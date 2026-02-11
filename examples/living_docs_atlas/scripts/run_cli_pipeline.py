#!/usr/bin/env python
"""Build docs inputs and render them via fullbleed CLI."""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Tuple


PROJECT_ROOT = Path(__file__).resolve().parents[1]
BUILD_DIR = PROJECT_ROOT / "build"
OUTPUT_DIR = PROJECT_ROOT / "output"
VENDOR_DIR = PROJECT_ROOT / "vendor"
VENDOR_CSS = VENDOR_DIR / "css"
VENDOR_FONTS = VENDOR_DIR / "fonts"

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
from build_inputs import build_inputs  # pylint: disable=wrong-import-position


def _resolve_fullbleed_cmd() -> List[str]:
    binary = shutil.which("fullbleed")
    if binary:
        return [binary]
    return [sys.executable, "-m", "fullbleed_cli.cli"]


def _supports_flag(base_cmd: List[str], flag: str) -> bool:
    proc = subprocess.run(base_cmd + ["--help"], capture_output=True, text=True, check=False)
    help_text = (proc.stdout or "") + "\n" + (proc.stderr or "")
    return flag in help_text


def _run_json(cmd: List[str], cwd: Path) -> Tuple[int, Dict, str]:
    proc = subprocess.run(cmd, cwd=str(cwd), capture_output=True, text=True, check=False)
    payload = {}
    stdout = (proc.stdout or "").strip()
    if stdout:
        for line in reversed(stdout.splitlines()):
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
                break
            except json.JSONDecodeError:
                continue
    return proc.returncode, payload, proc.stderr or ""


def _normalize_bytes_written(raw_value, pdf_path: Path) -> int | None:
    if isinstance(raw_value, int):
        return raw_value
    if isinstance(raw_value, list) and raw_value and isinstance(raw_value[0], int):
        return raw_value[0]
    if pdf_path.exists():
        return int(pdf_path.stat().st_size)
    return None


def _collect_asset_paths(installed_font_files: List[str]) -> List[Path]:
    assets: List[Path] = []
    bootstrap = _bootstrap_css_path()
    if bootstrap:
        assets.append(bootstrap)
    for rel in installed_font_files:
        path = PROJECT_ROOT / rel
        if path.exists():
            assets.append(path)
    return assets


def _bootstrap_css_path() -> Path | None:
    candidates = [
        VENDOR_CSS / "bootstrap.min.css",
        VENDOR_DIR / "bootstrap.min.css",
    ]
    for path in candidates:
        if path.exists():
            return path
    return None


def main() -> int:
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    build_result = build_inputs(font_url_prefix="../vendor/fonts", write_files=True)
    summary = build_result["summary"]

    html_path = BUILD_DIR / "atlas.html"
    css_path = BUILD_DIR / "atlas.css"
    out_pdf = OUTPUT_DIR / "living_docs_atlas.cli.pdf"
    verify_pdf = OUTPUT_DIR / "living_docs_atlas.verify.pdf"
    manifest_path = BUILD_DIR / "atlas.manifest.json"
    perf_path = BUILD_DIR / "atlas.perf.jsonl"
    jit_path = BUILD_DIR / "atlas.jit.jsonl"
    verify_perf_path = BUILD_DIR / "atlas.verify.perf.jsonl"
    verify_jit_path = BUILD_DIR / "atlas.verify.jit.jsonl"
    hash_path = BUILD_DIR / "atlas.sha256"

    base_cmd = _resolve_fullbleed_cmd()
    supports_emit_manifest = _supports_flag(base_cmd, "--emit-manifest")
    bootstrap_path = _bootstrap_css_path()
    asset_paths = _collect_asset_paths(summary.get("installed_font_files", []))

    render_cmd = base_cmd + [
        "--json",
        "render",
        "--html",
        str(html_path),
        "--css",
        str(css_path),
        "--out",
        str(out_pdf),
        "--emit-perf",
        str(perf_path),
        "--emit-jit",
        str(jit_path),
        "--deterministic-hash",
        str(hash_path),
    ]
    if supports_emit_manifest:
        render_cmd = base_cmd + [
            "--json",
            "--emit-manifest",
            str(manifest_path),
            "render",
            "--html",
            str(html_path),
            "--css",
            str(css_path),
            "--out",
            str(out_pdf),
            "--emit-perf",
            str(perf_path),
            "--emit-jit",
            str(jit_path),
            "--deterministic-hash",
            str(hash_path),
        ]

    if bootstrap_path:
        render_cmd += ["--css", str(bootstrap_path)]

    for asset in asset_paths:
        render_cmd += ["--asset", str(asset)]

    verify_cmd = base_cmd + [
        "--json",
        "verify",
        "--html",
        str(html_path),
        "--css",
        str(css_path),
        "--emit-pdf",
        str(verify_pdf),
        "--emit-perf",
        str(verify_perf_path),
        "--emit-jit",
        str(verify_jit_path),
    ]
    if bootstrap_path:
        verify_cmd += ["--css", str(bootstrap_path)]
    for asset in asset_paths:
        verify_cmd += ["--asset", str(asset)]

    render_rc, render_payload, render_stderr = _run_json(render_cmd, cwd=PROJECT_ROOT)
    verify_rc, verify_payload, verify_stderr = _run_json(verify_cmd, cwd=PROJECT_ROOT)
    normalized_render_bytes = _normalize_bytes_written(render_payload.get("bytes_written"), out_pdf)
    normalized_verify_bytes = _normalize_bytes_written(verify_payload.get("bytes_written"), verify_pdf)

    pipeline_summary = {
        "schema": "fullbleed.docs_cli_pipeline.v1",
        "render": {"returncode": render_rc, "payload": render_payload, "stderr": render_stderr},
        "verify": {"returncode": verify_rc, "payload": verify_payload, "stderr": verify_stderr},
        "inputs": {"html": str(html_path), "css": str(css_path)},
        "outputs": {
            "render_pdf": str(out_pdf),
            "verify_pdf": str(verify_pdf),
            "manifest": str(manifest_path) if supports_emit_manifest else None,
            "perf": str(perf_path),
            "jit": str(jit_path),
            "hash": str(hash_path),
        },
        "compat": {
            "supports_emit_manifest": supports_emit_manifest,
            "normalized_render_bytes_written": normalized_render_bytes,
            "normalized_verify_bytes_written": normalized_verify_bytes,
        },
        "asset_count": len(asset_paths),
    }

    summary_path = BUILD_DIR / "cli_pipeline_summary.json"
    summary_path.write_text(json.dumps(pipeline_summary, ensure_ascii=True, indent=2), encoding="utf-8")
    print(json.dumps(pipeline_summary, ensure_ascii=True, indent=2))

    if render_rc != 0 or verify_rc != 0:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
