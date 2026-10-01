#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise a real installed CLI watcher and retain its PDFs and result stream."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata, resources
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import tempfile
import threading

import fullbleed


def check(output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix="run-", dir=output)).resolve()
    html, css = workspace / "input.html", workspace / "style.css"
    pdf = workspace / "invoice.pdf"
    html.write_text("<h1>Watch initial</h1><div class='next'>Second page</div>", encoding="utf-8")
    css.write_text("@page { size: A4; margin: 20mm; } body { font-family: Inter; color: #173e38; }"
                   ".next { page-break-before: always; }", encoding="utf-8")
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    command = [sys.executable, "-I", "-m", "fullbleed", "--json-only", "render",
               "--html", str(html), "--css", str(css), "--asset", str(font),
               "--out", str(pdf), "--emit-image", str(workspace / "preview"),
               "--image-dpi", "72", "--emit-manifest", str(workspace / "manifest.json"),
               "--watch", "--watch-path", str(workspace),
               "--watch-interval", "0.05", "--watch-debounce", "0.1"]
    report = {"schema": "fullbleed.watch_smoke.v1", "ok": False,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "workspace": str(workspace), "python": sys.version,
              "fullbleed_version": metadata.version("fullbleed")}
    process = subprocess.Popen(command, cwd=workspace, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", bufsize=1)
    messages = queue.Queue()
    lines, errors, events = [], [], []

    def read_stdout():
        for line in process.stdout:
            lines.append(line.rstrip("\n"))
            messages.put(line)
        messages.put(None)

    def read_stderr():
        errors.extend(process.stderr.readlines())

    readers = [threading.Thread(target=read_stdout, daemon=True),
               threading.Thread(target=read_stderr, daemon=True)]
    for reader in readers:
        reader.start()

    def event():
        line = messages.get(timeout=30)
        if line is None:
            raise AssertionError(f"watcher exited early: {process.poll()}; {''.join(errors)}")
        result = json.loads(line)
        events.append(result)
        if result.get("ok"):
            # Store each successful stage outside the watched workspace.
            stage = output / f"stage-{len(events)}"
            stage.mkdir(exist_ok=True)
            (stage / "invoice.pdf").write_bytes(pdf.read_bytes())
            for image_path in result["outputs"].get("image_paths") or []:
                image = Path(image_path)
                (stage / image.name).write_bytes(image.read_bytes())
        return result

    def quiet():
        try:
            extra = messages.get(timeout=0.5)
        except queue.Empty:
            return
        raise AssertionError(f"unexpected rebuild without an input edit: {extra!r}")

    try:
        first = event()
        assert first["ok"] is True
        assert len(first["outputs"]["image_paths"]) == 2
        assert fullbleed.inspect_pdf(str(pdf))["page_count"] == 2
        first_hash = first["outputs"]["sha256"]
        quiet()

        css.write_text(css.read_text(encoding="utf-8").replace("#173e38", "#b82057"),
                       encoding="utf-8")
        styled = event()
        assert styled["ok"] is True and styled["outputs"]["sha256"] != first_hash
        quiet()

        before_missing = pdf.read_bytes()
        html.unlink()
        missing = event()
        assert missing["ok"] is False and missing["schema"] == "fullbleed.error.v1"
        assert process.poll() is None and pdf.read_bytes() == before_missing

        replacement = workspace / "save.tmp"
        replacement.write_text("<h1>Watch recovered</h1><p>Final single-page invoice.</p>",
                               encoding="utf-8")
        replacement.replace(html)
        recovered = event()
        assert recovered["ok"] is True
        assert fullbleed.inspect_pdf(str(pdf))["page_count"] == 1
        assert len(recovered["outputs"]["image_paths"]) == 1
        assert not Path(first["outputs"]["image_paths"][1]).exists()
        quiet()

        # An extra watched dependency triggers a pass even when HTML/CSS stay the same.
        dependency = workspace / "nested" / "record.json"
        dependency.parent.mkdir()
        dependency.write_text('{"revision": 1}', encoding="utf-8")
        additional = event()
        assert additional["ok"] is True
        assert additional["outputs"]["sha256"] == recovered["outputs"]["sha256"]
        quiet()
        assert not errors
        report.update(ok=True, event_count=len(events), checks=[
            "initial_two_page_pdf_and_previews", "css_edit_changes_pdf",
            "output_files_do_not_retrigger", "missing_input_reports_json_error",
            "restored_input_recovers", "old_preview_removed_when_page_count_shrinks",
            "new_recursive_dependency_rebuilds", "json_only_stderr_empty",
        ], final_pdf_sha256=hashlib.sha256(pdf.read_bytes()).hexdigest())
    except BaseException as exc:
        report["error"] = str(exc)
        raise
    finally:
        if process.poll() is None:
            if os.name == "nt":
                process.terminate()
                report["stop"] = "harness terminated Windows child; Ctrl-C covered by unit tests"
            else:
                process.send_signal(signal.SIGINT)
                report["stop"] = "SIGINT"
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
                report["ok"] = False
                report["stop"] = "forced kill after stop timeout"
        for reader in readers:
            reader.join(timeout=2)
        report["exit_code"] = process.returncode
        if os.name != "nt" and process.returncode != 130:
            report["ok"] = False
        process.stdout.close()
        process.stderr.close()
        (output / "events.jsonl").write_text("\n".join(lines) + "\n", encoding="utf-8")
        (output / "stderr.txt").write_text("".join(errors), encoding="utf-8")
        (output / "verification.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    assert report["ok"], report
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/watch-smoke"))
    options = parser.parse_args()
    print(json.dumps(check(options.out.resolve()), indent=2))
