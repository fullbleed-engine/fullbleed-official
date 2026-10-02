#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify the downloadable starter without changing its reviewed baseline."""
import argparse
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("output/pdf-regression-verification"))
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parent
    workspace = Path(tempfile.mkdtemp(prefix="project with spaces ", dir=out))
    project = workspace / "invoice"
    shutil.copytree(root, project, ignore=shutil.ignore_patterns("output", "__pycache__", "*.pyc"))
    original_record = (project / "baseline.json").read_bytes()
    original_pdf = (project / "baseline.pdf").read_bytes()
    checks = []

    def run(name, action="check", codes=()):
        result = subprocess.run([sys.executable, "-I", str(project / "check.py"), action],
                                cwd=workspace, capture_output=True, text=True, encoding="utf-8", timeout=45)
        folder = out / name
        shutil.copytree(project / "output", folder, dirs_exist_ok=True)
        (folder / "stdout.txt").write_text(result.stdout, encoding="utf-8")
        (folder / "process-stderr.txt").write_text(result.stderr, encoding="utf-8")
        report = json.loads((folder / "render.json").read_text(encoding="utf-8"))
        assert result.returncode == (1 if codes else 0), (name, report)
        assert {f["code"] for f in report.get("failures", [])} == set(codes), (name, report)
        assert report["ok"] is (not bool(codes))
        assert report["outputs"]["fallbacks"]["missing_glyphs"] == 0
        pdf = (folder / "invoice.pdf").read_bytes()
        assert hashlib.sha256(pdf).hexdigest() == report["outputs"]["sha256"]
        assert len(report["outputs"]["image_paths"]) == 1
        checks.append({"case": name, "exit_code": result.returncode, "codes": sorted(codes),
                       "pdf_sha256": report["outputs"]["sha256"]})
        return pdf

    assert run("unchanged-relocated") == original_pdf
    css = (project / "style.css").read_text(encoding="utf-8")
    (project / "style.css").write_text(css + "\nh1 { color: #bc3022; }\n", encoding="utf-8")
    assert run("changed-style", codes=["REPRO_INPUT_DRIFT", "REPRO_HASH_MISMATCH"]) != original_pdf
    assert (project / "baseline.json").read_bytes() == original_record
    assert (project / "baseline.pdf").read_bytes() == original_pdf
    accepted = run("record-intentional-change", "record")
    assert (project / "baseline.pdf").read_bytes() == accepted
    assert run("accepted-change") == accepted
    accepted_record = (project / "baseline.json").read_bytes()
    (project / "baseline.json").write_text("{}", encoding="utf-8")
    run("malformed-baseline", codes=["REPRO_RECORD_INVALID"])
    assert (project / "baseline.json").read_text() == "{}"
    (project / "baseline.json").write_bytes(accepted_record)
    assert run("recovered") == accepted
    assert (root / "baseline.json").read_bytes() == original_record
    assert (root / "baseline.pdf").read_bytes() == original_pdf
    report = {"ok": True, "platform": sys.platform, "python": sys.version, "engine": version("fullbleed"),
              "cases": checks, "source_baseline_unchanged": True,
              "scope": "This invoice and six starter workflows, not general visual or standards validation."}
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
