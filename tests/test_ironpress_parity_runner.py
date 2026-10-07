from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
from types import SimpleNamespace
import zipfile

import pytest


def load_runner():
    path = Path(__file__).resolve().parents[1] / "tools" / "run_ironpress_parity.py"
    spec = importlib.util.spec_from_file_location("ironpress_runner_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def make_wheel(root, *, version="2.5.14", platform="manylinux_2_17_x86_64.manylinux2014_x86_64", name="fullbleed", metadata_version=None):
    wheel = root / f"fullbleed-{version}-cp310-abi3-{platform}.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr(
            f"fullbleed-{version}.dist-info/METADATA",
            f"Metadata-Version: 2.1\nName: {name}\nVersion: {metadata_version or version}\n",
        )
    return wheel


@pytest.mark.parametrize("version", ["2.2.3", "2.5.14", "3.0.0rc1"])
def test_release_wheels_are_not_pinned_to_an_old_version(tmp_path, version):
    runner = load_runner()
    wheel = make_wheel(tmp_path, version=version)
    assert runner.discover_wheel(tmp_path, wheel) == wheel
    assert runner.wheel_identity(wheel) == {
        "filename": wheel.name,
        "version": version,
        "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
    }


@pytest.mark.parametrize("platform", ["win_amd64", "manylinux_2_17_aarch64", "musllinux_1_2_x86_64", "manylinux_2_17_x86_64.win_x86_64"])
def test_other_abis_and_platforms_remain_rejected(tmp_path, platform):
    with pytest.raises(RuntimeError, match="not a compatible"):
        load_runner().discover_wheel(tmp_path, make_wheel(tmp_path, platform=platform))


@pytest.mark.parametrize("kwargs", [{"name": "another-project"}, {"metadata_version": "2.2.3"}])
def test_renamed_or_misversioned_archives_are_rejected(tmp_path, kwargs):
    with pytest.raises(RuntimeError, match="metadata disagree"):
        load_runner().discover_wheel(tmp_path, make_wheel(tmp_path, **kwargs))


def test_wheel_outside_mounted_repository_is_rejected(tmp_path):
    root = tmp_path / "repo"
    root.mkdir()
    with pytest.raises(RuntimeError, match="inside the FullBleed repository"):
        load_runner().discover_wheel(root, make_wheel(tmp_path))


def write_report(root, *, invocation="fresh-run", change=None):
    fixtures = [
        {"id": f"fixture-{i}", "category": "layout", "status": "PASS" if i < 1642 else "REFERENCE-DISPUTED"}
        for i in range(1662)
    ]
    report = {
        "invocation_id": invocation, "run_complete": True,
        "overall": {"pass": 1642, "fail": 0, "reference_disputed": 20, "total": 1662},
        "categories": [{"features": [{"fixtures": fixtures}]}],
    }
    if change:
        change(report)
    raw = json.dumps(report).encode()
    root.mkdir(parents=True, exist_ok=True)
    (root / "report.json").write_bytes(raw)
    digest = hashlib.sha256(raw).hexdigest()
    (root / "REPORT.md").write_text(
        f"<!-- parity-invocation-id: {invocation} -->\n<!-- parity-report-json-sha256: {digest} -->\n",
        encoding="utf-8",
    )
    (root / "reports").mkdir(exist_ok=True)
    (root / "reports/index.html").write_text(
        f'<meta name="parity-invocation-id" content="{invocation}">'
        f'<meta name="parity-report-json-sha256" content="{digest}">', encoding="utf-8",
    )


def test_current_complete_report_keeps_disputes_separate(tmp_path):
    write_report(tmp_path)
    checked = load_runner().check_full_report(tmp_path, "fresh-run")
    assert checked["verified_complete"] is True
    assert checked["overall"] == {"pass": 1642, "fail": 0, "reference_disputed": 20, "total": 1662}


@pytest.mark.parametrize("change", [
    lambda r: r.update(invocation_id="previous-run"),
    lambda r: r.update(run_complete=False),
    lambda r: r["overall"].update(**{"pass": 1662, "reference_disputed": 0}),
    lambda r: r["categories"][0]["features"][0]["fixtures"].pop(),
    lambda r: r["categories"][0]["features"][0]["fixtures"][0].update(id="fixture-1"),
])
def test_stale_truncated_and_miscounted_reports_cannot_verify(tmp_path, change):
    write_report(tmp_path, change=change)
    assert load_runner().check_full_report(tmp_path, "fresh-run")["verified_complete"] is False


def test_mixed_generation_html_is_rejected(tmp_path):
    write_report(tmp_path)
    (tmp_path / "reports/index.html").write_text("old gallery", encoding="utf-8")
    checked = load_runner().check_full_report(tmp_path, "fresh-run")
    assert checked["verified_complete"] is False
    assert "reports/index.html is not bound to the same report" in checked["problems"]


def test_missing_report_is_rejected(tmp_path):
    assert load_runner().check_full_report(tmp_path, "fresh-run")["verified_complete"] is False


def test_exported_gallery_retains_linked_images_and_requested_pdfs(tmp_path, monkeypatch):
    runner = load_runner()
    source = tmp_path / "container"
    source.mkdir()
    write_report(source)
    for name in ["refs", "out", "diffs", "pdfs"]:
        (source / name / "layout").mkdir(parents=True)
        (source / name / "layout/fixture.bin").write_bytes(name.encode())
    with (source / "reports/index.html").open("a", encoding="utf-8") as stream:
        stream.write('<img src="../refs/layout/fixture.bin"><img src="../out/layout/fixture.bin">')

    def docker_copy(command, **kwargs):
        if command[:2] == ["docker", "cp"]:
            name = command[2].rsplit("/", 1)[-1]
            if name == "LICENSE":
                Path(command[3]).write_text("Upstream MIT license")
                return SimpleNamespace(returncode=0)
            destination = Path(command[3]) / name
            if (source / name).is_dir():
                shutil.copytree(source / name, destination)
            else:
                shutil.copy2(source / name, destination)
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(runner, "output", lambda *a, **kw: "test-container")
    monkeypatch.setattr(runner, "run", docker_copy)
    exported = tmp_path / "exported"
    assert runner.copy_evidence("volume", exported, keep_pdfs=True) == []
    assert (exported / "reports/../refs/layout/fixture.bin").read_bytes() == b"refs"
    assert (exported / "reports/../out/layout/fixture.bin").read_bytes() == b"out"
    assert (exported / "pdfs/layout/fixture.bin").read_bytes() == b"pdfs"
    assert runner.check_full_report(exported, "fresh-run")["verified_complete"] is True
    runner.write_run_manifest(exported, {"gate_passed": False})
    manifest = json.loads((exported / "manifest.json").read_text())
    for name, record in manifest["files"].items():
        data = (exported / name).read_bytes()
        assert record == {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def test_nonempty_evidence_directory_is_never_reused(tmp_path, monkeypatch):
    runner = load_runner()
    wheel = make_wheel(tmp_path)
    evidence = tmp_path / "evidence"
    evidence.mkdir()
    (evidence / "previous.txt").write_text("retained evidence")
    monkeypatch.setattr(runner, "repository_root", lambda: tmp_path)
    monkeypatch.setattr(runner.shutil, "which", lambda _: "available")
    with pytest.raises(RuntimeError, match="evidence directory must be empty"):
        runner.main(["--wheel", str(wheel), "--evidence-dir", str(evidence)])
    assert (evidence / "previous.txt").read_text() == "retained evidence"
