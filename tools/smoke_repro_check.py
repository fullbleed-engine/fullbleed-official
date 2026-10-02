#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise reproducibility gates through an isolated installed CLI process."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata, resources
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import fullbleed


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    schema_result = subprocess.run(
        [sys.executable, "-I", "-m", "fullbleed", "--schema", "repro-record"],
        check=True, capture_output=True, text=True, encoding="utf-8", timeout=45,
    )
    schema = json.loads(schema_result.stdout)
    assert schema["target"] == "fullbleed.repro_record.v1"
    assert set(schema["definition"]["required"]) == {"schema", "input_fingerprint_sha256", "output_pdf_sha256"}
    (out / "record-schema.json").write_text(schema_result.stdout, encoding="utf-8")
    project = Path(tempfile.mkdtemp(prefix="project-", dir=out)).resolve()
    source = "<main><h1>Baseline invoice</h1><p>Invoice CI-1042</p><section><h2>Terms</h2><p>Second page retained.</p></section></main>"
    css = "@page{size:A4;margin:20mm}body{font-family:Inter;color:#173e38}h1{font-size:30pt}section{break-before:page}"
    (project / "input.html").write_text(source, encoding="utf-8")
    (project / "style.css").write_text(css, encoding="utf-8")
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    (project / "font.ttf").write_bytes(font.read_bytes())
    base = [sys.executable, "-I", "-m", "fullbleed", "--json-only", "render",
            "--html", "input.html", "--css", "style.css", "--asset", "font.ttf",
            "--out", "document.pdf", "--emit-image", "preview", "--image-dpi", "72",
            "--fail-on", "missing-glyphs"]
    results = []

    def run(name, flags, codes=(), cwd=project, operation="render"):
        command = list(base)
        if operation == "verify":
            command[command.index("render")] = "verify"
            command[command.index("--out")] = "--emit-pdf"
        p = subprocess.run([*command, *flags], cwd=cwd, capture_output=True,
                           text=True, encoding="utf-8", timeout=45)
        folder = out / name
        folder.mkdir(exist_ok=True)
        (folder / "result.json").write_text(p.stdout, encoding="utf-8")
        (folder / "stderr.txt").write_text(p.stderr, encoding="utf-8")
        result = json.loads(p.stdout)
        observed = {f["code"] for f in result.get("failures", [])}
        assert p.returncode == (1 if codes else 0), (name, p.returncode, result)
        assert result["schema"] == f"fullbleed.{operation}_result.v1", (name, result)
        assert result["ok"] is (not bool(codes)), (name, result)
        assert observed == set(codes), (name, observed, codes)
        assert result["outputs"]["repro_status"] == ("fail" if codes else "pass")
        pdf = cwd / "document.pdf"
        assert pdf.is_file() and pdf.read_bytes().startswith(b"%PDF-")
        assert len(fullbleed.extract_pdf_page_texts(str(pdf))["pages"]) == 2
        shutil.copyfile(pdf, folder / "document.pdf")
        previews = result["outputs"]["image_paths"]
        assert len(previews) == 2
        for image in previews:
            shutil.copyfile(cwd / image, folder / Path(image).name)
        assert all(f["recommended_actions"] for f in result.get("failures", []))
        results.append({"case": name, "returncode": p.returncode, "codes": sorted(observed),
                        "pdf_sha256": hashlib.sha256(pdf.read_bytes()).hexdigest(), "pages": 2})
        return result

    run("record", ["--repro-record", "baseline.json"])
    baseline = (project / "baseline.json").read_bytes()
    run("unchanged", ["--repro-check", "baseline.json"])
    relocated = out / "copied project with spaces"
    shutil.copytree(project, relocated, dirs_exist_ok=True)
    run("relocated", ["--repro-check", "baseline.json"], cwd=relocated)
    assert results[0]["pdf_sha256"] == results[1]["pdf_sha256"] == results[2]["pdf_sha256"]

    (project / "style.css").write_text(css.replace("#173e38", "#9b2535"), encoding="utf-8")
    run("changed-style", ["--repro-check", "baseline.json", "--repro-record", "candidate.json"],
        ["REPRO_INPUT_DRIFT", "REPRO_HASH_MISMATCH"])
    assert (project / "baseline.json").read_bytes() == baseline
    assert (project / "candidate.json").read_bytes() != baseline
    run("conflicting-output", ["--repro-check", "baseline.json", "--repro-record", "./baseline.json"],
        ["REPRO_RECORD_CONFLICT"])
    assert (project / "baseline.json").read_bytes() == baseline
    (project / "style.css").write_text(css, encoding="utf-8")

    for name, record in [("empty-object", {}), ("array", []), ("null", None),
                         ("missing-hashes", {"schema": "fullbleed.repro_record.v1"}),
                         ("wrong-schema", dict(json.loads(baseline), schema="unknown")),
                         ("invalid-digest", dict(json.loads(baseline), output_pdf_sha256="")),
                         ("invalid-lock", dict(json.loads(baseline), assets_lock=[]))]:
        (project / "invalid.json").write_text(json.dumps(record), encoding="utf-8")
        run(name, ["--repro-check", "invalid.json"], ["REPRO_RECORD_INVALID"])
    (project / "invalid.json").write_bytes(b"\xff\xfe")
    run("invalid-utf8", ["--repro-check", "invalid.json"], ["REPRO_RECORD_INVALID"])
    run("missing-file", ["--repro-check", "absent.json"], ["REPRO_RECORD_NOT_FOUND"])
    assert not (project / "absent.json").exists()

    (project / "assets.lock.json").write_text('{"fixture":"first"}', encoding="utf-8")
    run("added-lock", ["--repro-check", "baseline.json"], ["REPRO_LOCK_MISMATCH"])
    run("record-lock", ["--repro-record", "locked.json"])
    (project / "assets.lock.json").write_text('{"fixture":"changed"}', encoding="utf-8")
    run("changed-lock", ["--repro-check", "locked.json"], ["REPRO_LOCK_MISMATCH"])
    (project / "assets.lock.json").unlink()
    run("removed-lock", ["--repro-check", "locked.json"], ["REPRO_LOCK_MISMATCH"])
    run("restored", ["--repro-check", "baseline.json"])
    assert (project / "baseline.json").read_bytes() == baseline
    assert results[-1]["pdf_sha256"] == results[0]["pdf_sha256"]
    run("verify-record", ["--repro-record", "verify-baseline.json"], operation="verify")
    run("verify-unchanged", ["--repro-check", "verify-baseline.json"], operation="verify")
    run("verify-invalid", ["--repro-check", "invalid.json"], ["REPRO_RECORD_INVALID"], operation="verify")
    report = {"schema": "fullbleed.repro_smoke.v1", "ok": True,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "fullbleed_version": metadata.version("fullbleed"), "platform": sys.platform,
              "python": sys.version, "cases": results,
              "baseline_unchanged": True, "relocated_project_passes": True,
              "scope": "These CLI fixtures and failure controls; no standards-conformance claim."}
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/repro-smoke"))
    args = parser.parse_args()
    report = check(args.out.resolve())
    print(json.dumps({"ok": report["ok"], "cases": len(report["cases"]),
                      "fullbleed_version": report["fullbleed_version"], "out": str(args.out)}))
