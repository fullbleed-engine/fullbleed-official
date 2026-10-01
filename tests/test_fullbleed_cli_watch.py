from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from fullbleed_cli import cli, watch


class Clock:
    def __init__(self, monkeypatch, actions, stop=6):
        self.now = 0.0
        self.ticks = 0
        self.actions = actions
        self.stop = stop
        monkeypatch.setattr(watch, "time", self)

    def monotonic(self):
        return self.now

    def sleep(self, interval):
        self.now += interval
        self.ticks += 1
        if self.ticks >= self.stop:
            raise KeyboardInterrupt
        action = self.actions.get(self.ticks)
        if action:
            action()


def args_for(tmp_path, *extra):
    args = cli._build_parser().parse_args([
        "--json-only", "render", "--html", str(tmp_path / "input.html"),
        "--out", str(tmp_path / "output.pdf"), "--watch",
        "--watch-interval", "0.1", "--watch-debounce", "0", *extra,
    ])
    cli._apply_global_flags(args)
    return args


def run_until_stopped(args, render, errors):
    with pytest.raises(SystemExit) as exc:
        watch.run(args, render, lambda current, error: errors.append(str(error)))
    assert exc.value.code == 130


def test_watch_coalesces_burst_and_ignores_its_outputs(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("first", encoding="utf-8")
    args = args_for(tmp_path, "--watch-path", str(tmp_path),
                    "--watch-debounce", "0.15", "--emit-image", str(tmp_path / "png"),
                    "--emit-manifest", str(tmp_path / "manifest.json"),
                    "--profile", "preflight")
    monkeypatch.chdir(tmp_path)
    Clock(monkeypatch, {
        1: lambda: html.write_text("partial save", encoding="utf-8"),
        2: lambda: html.write_text("final save", encoding="utf-8"),
    }, stop=8)
    rendered = []

    def render(current):
        rendered.append(html.read_text(encoding="utf-8"))
        assert current.watch is False and current.watch_path is None
        Path(current.out).write_bytes(b"output")
        Path(current.emit_manifest).write_text(str(len(rendered)))
        Path("fullbleed_preflight.jit").write_text(str(len(rendered)))
        Path("fullbleed_preflight.perf").write_text(str(len(rendered)))
        Path(current.emit_image).mkdir(exist_ok=True)
        (Path(current.emit_image) / "output_page1.png").write_bytes(b"preview")
        current.document_timestamp = "changed by renderer"

    errors = []
    run_until_stopped(args, render, errors)
    assert rendered == ["first", "final save"]
    assert errors == []
    assert args.document_timestamp is None


def test_watch_keeps_changes_made_during_render(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("first", encoding="utf-8")
    Clock(monkeypatch, {}, stop=4)
    rendered = []

    def render(current):
        rendered.append(html.read_text(encoding="utf-8"))
        if len(rendered) == 1:
            html.write_text("saved while rendering", encoding="utf-8")

    run_until_stopped(args_for(tmp_path), render, [])
    assert rendered == ["first", "saved while rendering"]


def test_watch_recovers_after_delete_and_atomic_save(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("first", encoding="utf-8")

    def replace():
        replacement = tmp_path / "replacement.tmp"
        replacement.write_text("restored", encoding="utf-8")
        replacement.replace(html)

    Clock(monkeypatch, {1: html.unlink, 2: replace}, stop=5)
    rendered, errors = [], []
    run_until_stopped(args_for(tmp_path),
                      lambda args: rendered.append(html.read_text(encoding="utf-8")), errors)
    assert rendered == ["first", "restored"]
    assert len(errors) == 1


def test_watch_recovers_after_quality_gate_failure(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("first", encoding="utf-8")
    Clock(monkeypatch, {1: lambda: html.write_text("second", encoding="utf-8")}, stop=4)
    attempts = []

    def render(args):
        attempts.append(html.read_text(encoding="utf-8"))
        if len(attempts) == 1:
            raise SystemExit(1)

    errors = []
    run_until_stopped(args_for(tmp_path), render, errors)
    assert attempts == ["first", "second"]
    assert errors == []  # The render handler already emitted the failed result.


def test_watch_removes_only_unmodified_previews_from_this_session(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("three pages", encoding="utf-8")
    images = [tmp_path / f"page{number}.png" for number in range(1, 4)]
    untouched = tmp_path / "unrelated.png"
    untouched.write_bytes(b"keep")

    def change():
        html.write_text("one page", encoding="utf-8")
        images[2].write_bytes(b"edited by someone else")

    Clock(monkeypatch, {1: change}, stop=4)
    calls = []

    def render(args):
        calls.append(True)
        current = images if len(calls) == 1 else images[:1]
        for path in current:
            path.write_bytes(b"preview")
        return {"image_paths": [str(path) for path in current]}

    run_until_stopped(args_for(tmp_path), render, [])
    assert images[0].exists()
    assert not images[1].exists()
    assert images[2].read_bytes() == b"edited by someone else"
    assert untouched.read_bytes() == b"keep"


@pytest.mark.parametrize("extra", [
    ["--out", "-"], ["--html", "-"], ["--css", "-"], ["--emit-manifest", "-"],
    ["--watch-interval", "0"], ["--watch-interval", "nan"],
    ["--watch-interval", "inf"], ["--watch-debounce", "-1"],
    ["--watch-debounce", "nan"],
])
def test_watch_rejects_invalid_configuration_before_render(tmp_path, extra):
    args = args_for(tmp_path, *extra)
    with pytest.raises(ValueError):
        watch.run(args, lambda _: pytest.fail("rendered invalid configuration"), lambda *_: None)
    assert not (tmp_path / "output.pdf").exists()


def test_watch_rejects_input_output_overlap(tmp_path):
    html = tmp_path / "input.html"
    html.write_text("keep this source", encoding="utf-8")
    for extra in (["--out", str(html)], ["--emit-image", str(tmp_path)]):
        with pytest.raises(ValueError, match="overlaps"):
            watch.WatchInputs(args_for(tmp_path, *extra), [])
    assert html.read_text(encoding="utf-8") == "keep this source"


def test_inline_html_needs_a_local_watch_input(tmp_path):
    with pytest.raises(ValueError, match="local input"):
        watch.WatchInputs(args_for(tmp_path, "--html-str", "<p>Hi</p>"), [])
    watched = watch.WatchInputs(args_for(tmp_path, "--html-str", "<p>Hi</p>",
                                         "--watch-path", str(tmp_path / "future.css")), [])
    assert len(watched.roots) == 1


def test_recursive_watch_sees_new_assets_but_ignores_cache(tmp_path):
    assets = tmp_path / "assets"
    assets.mkdir()
    watched = watch.WatchInputs(args_for(tmp_path, "--watch-path", str(assets)), [])
    initial = watched.snapshot()
    cache = assets / "__pycache__"
    cache.mkdir()
    (cache / "compiled.pyc").write_bytes(b"cache")
    assert watched.snapshot() == initial
    (assets / "new.svg").write_text("<svg/>", encoding="utf-8")
    assert watched.snapshot() != initial


def test_watch_rebuilds_after_asset_and_file_backed_options_change(tmp_path, monkeypatch):
    html = tmp_path / "input.html"
    html.write_text("<h1>Invoice</h1>", encoding="utf-8")
    asset = tmp_path / "brand.svg"
    asset.write_text("first asset", encoding="utf-8")
    margins = tmp_path / "margins.json"
    margins.write_text('{"1": {"top": "10mm"}}', encoding="utf-8")
    args = args_for(tmp_path, "--asset", str(asset), "--page-margins", str(margins))
    Clock(monkeypatch, {
        1: lambda: asset.write_text("replacement asset", encoding="utf-8"),
        2: lambda: margins.write_text('{"1": {"top": "20mm"}}', encoding="utf-8"),
    }, stop=5)
    seen = []
    with pytest.raises(SystemExit) as exc:
        watch.run(args, lambda _: seen.append((asset.read_text(), margins.read_text())),
                  lambda *_: pytest.fail("unexpected watch error"), asset_paths=[str(asset)])
    assert exc.value.code == 130
    assert [asset_text for asset_text, _ in seen] == ["first asset", "replacement asset", "replacement asset"]
    assert seen[-1][1] == '{"1": {"top": "20mm"}}'


def test_watch_path_without_watch_reports_configuration_error(tmp_path, capsys):
    result = cli.main(["--json-only", "render", "--html-str", "<p>Hello</p>",
                       "--out", str(tmp_path / "out.pdf"), "--watch-path", str(tmp_path)])
    assert result == 3
    assert "--watch-path requires --watch" in json.loads(capsys.readouterr().out)["message"]
    assert not (tmp_path / "out.pdf").exists()


def test_watched_native_render_streams_results_and_recovers(tmp_path, monkeypatch, capsys):
    html = tmp_path / "input.html"
    html.write_text("<h1>First invoice</h1>", encoding="utf-8")
    Clock(monkeypatch, {
        1: html.unlink,
        2: lambda: html.write_text("<h1>Rebuilt invoice</h1>", encoding="utf-8"),
    }, stop=5)
    args = args_for(tmp_path, "--emit-manifest", str(tmp_path / "manifest.json"))
    with pytest.raises(SystemExit) as exc:
        cli._execute_command(args)
    assert exc.value.code == 130
    captured = capsys.readouterr()
    results = [json.loads(line) for line in captured.out.splitlines()]
    assert [result["ok"] for result in results] == [True, False, True]
    assert results[1]["schema"] == "fullbleed.error.v1"
    assert results[0]["outputs"]["sha256"] != results[2]["outputs"]["sha256"]
    assert captured.err == ""
    assert (tmp_path / "output.pdf").read_bytes().startswith(b"%PDF-")
    assert json.loads((tmp_path / "manifest.json").read_text())["output"]["path"] == args.out
