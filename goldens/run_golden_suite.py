#!/usr/bin/env python3
"""Public golden regression suite for Fullbleed render outputs.

Outputs:
- Per-case PDFs and page PNGs under `goldens/output/<mode>/<case>/`
- Committed expected PNG baselines under `goldens/expected/png/<case>/`
- Committed hash contract at `goldens/expected/golden_suite.expected.json`
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import shlex
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parent
CASES_ROOT = ROOT / "cases"
EXPECTED_ROOT = ROOT / "expected"
EXPECTED_PNG_ROOT = EXPECTED_ROOT / "png"
EXPECTED_MANIFEST = EXPECTED_ROOT / "golden_suite.expected.json"

BOOTSTRAP_CSS = ROOT.parent / "python" / "fullbleed_assets" / "bootstrap.min.css"
NOTO_FONT = ROOT.parent / "python" / "fullbleed_assets" / "fonts" / "NotoSans-Regular.ttf"


@dataclass(frozen=True)
class CaseSpec:
    name: str
    description: str
    html: Path
    css: Path


CASES: tuple[CaseSpec, ...] = (
    CaseSpec(
        name="invoice",
        description="One-page invoice with bill-to card, line item table, and totals block.",
        html=CASES_ROOT / "invoice" / "invoice.html",
        css=CASES_ROOT / "invoice" / "invoice.css",
    ),
    CaseSpec(
        name="statement",
        description="Account statement with summary panels and running-balance transaction table.",
        html=CASES_ROOT / "statement" / "statement.html",
        css=CASES_ROOT / "statement" / "statement.css",
    ),
    CaseSpec(
        name="menu",
        description="Restaurant menu with two-column sections and pricing hierarchy.",
        html=CASES_ROOT / "menu" / "menu.html",
        css=CASES_ROOT / "menu" / "menu.css",
    ),
)


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def _resolve_cli(override: str | None) -> list[str]:
    if override:
        return shlex.split(override, posix=(os.name != "nt"))
    found = shutil.which("fullbleed")
    if found:
        return [found]
    if importlib.util.find_spec("fullbleed") is not None:
        return [sys.executable, "-m", "fullbleed"]
    if importlib.util.find_spec("fullbleed_cli.cli") is not None:
        return [sys.executable, "-m", "fullbleed_cli.cli"]
    raise RuntimeError(
        "Unable to find Fullbleed CLI. Install package (`python -m pip install fullbleed`) "
        "or pass --cli with an explicit command."
    )


def _select_cases(names: Iterable[str] | None) -> list[CaseSpec]:
    if not names:
        return list(CASES)
    wanted = {name.strip().lower() for name in names}
    selected = [case for case in CASES if case.name in wanted]
    if not selected:
        raise ValueError(f"No matching cases for --case {sorted(wanted)}")
    missing = sorted(wanted.difference({case.name for case in selected}))
    if missing:
        raise ValueError(f"Unknown case names: {missing}")
    return selected


def _run_render(
    case: CaseSpec,
    cli_cmd: list[str],
    run_root: Path,
    image_dpi: int,
) -> dict:
    case_root = run_root / case.name
    pages_dir = case_root / "pages"
    case_root.mkdir(parents=True, exist_ok=True)
    pages_dir.mkdir(parents=True, exist_ok=True)

    out_pdf = case_root / f"{case.name}.pdf"
    out_hash = case_root / f"{case.name}.sha256"
    out_manifest = case_root / f"{case.name}.manifest.json"
    out_repro = case_root / f"{case.name}.repro.json"

    cmd = [
        *cli_cmd,
        "--json-only",
        "render",
        "--profile",
        "preflight",
        "--html",
        str(case.html),
        "--css",
        str(case.css),
        "--asset",
        str(BOOTSTRAP_CSS),
        "--asset-kind",
        "css",
        "--asset-name",
        "bootstrap",
        "--asset",
        str(NOTO_FONT),
        "--asset-kind",
        "font",
        "--asset-name",
        "noto-sans",
        "--emit-image",
        str(pages_dir),
        "--image-dpi",
        str(image_dpi),
        "--emit-manifest",
        str(out_manifest),
        "--deterministic-hash",
        str(out_hash),
        "--repro-record",
        str(out_repro),
        "--out",
        str(out_pdf),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise RuntimeError(
            f"[{case.name}] render failed with exit={proc.returncode}\n"
            f"STDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}"
        )
    try:
        payload = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"[{case.name}] expected JSON from CLI but got:\n{proc.stdout}\nSTDERR:\n{proc.stderr}"
        ) from exc
    if not payload.get("ok", False):
        raise RuntimeError(f"[{case.name}] CLI returned ok=false payload:\n{json.dumps(payload, indent=2)}")

    png_files = sorted(pages_dir.glob("*.png"))
    if not png_files:
        raise RuntimeError(f"[{case.name}] no PNG pages emitted to {pages_dir}")

    pdf_sha = _sha256_file(out_pdf)
    deterministic_text = out_hash.read_text(encoding="utf-8").strip()
    if deterministic_text and deterministic_text != pdf_sha:
        raise RuntimeError(
            f"[{case.name}] deterministic hash mismatch: file={deterministic_text} computed={pdf_sha}"
        )

    png_records = [
        {
            "file": png.name,
            "sha256": _sha256_file(png),
            "bytes": png.stat().st_size,
        }
        for png in png_files
    ]

    return {
        "case": case.name,
        "description": case.description,
        "inputs": {
            "html": str(case.html.relative_to(ROOT.parent)).replace("\\", "/"),
            "css": str(case.css.relative_to(ROOT.parent)).replace("\\", "/"),
        },
        "artifacts": {
            "pdf": {
                "file": out_pdf.name,
                "sha256": pdf_sha,
                "bytes": out_pdf.stat().st_size,
            },
            "png_pages": png_records,
            "deterministic_hash_file": out_hash.name,
            "manifest_file": out_manifest.name,
            "repro_file": out_repro.name,
        },
        "render": {
            "schema": payload.get("schema"),
            "ok": bool(payload.get("ok")),
            "warnings": payload.get("warnings", []),
        },
    }


def _write_expected(records: list[dict], run_root: Path) -> None:
    EXPECTED_ROOT.mkdir(parents=True, exist_ok=True)
    EXPECTED_PNG_ROOT.mkdir(parents=True, exist_ok=True)

    cases_out: list[dict] = []
    for record in records:
        case_name = record["case"]
        case_run_pages = run_root / case_name / "pages"
        case_expected_pages = EXPECTED_PNG_ROOT / case_name
        case_expected_pages.mkdir(parents=True, exist_ok=True)

        for png in case_run_pages.glob("*.png"):
            shutil.copy2(png, case_expected_pages / png.name)

        case_copy = dict(record)
        case_copy["expected_png_dir"] = str(case_expected_pages.relative_to(ROOT)).replace("\\", "/")
        cases_out.append(case_copy)

    manifest = {
        "schema": "fullbleed.golden_suite.v1",
        "suite": "public_showcase",
        "assets": {
            "font": str(NOTO_FONT.relative_to(ROOT.parent)).replace("\\", "/"),
            "bootstrap_css": str(BOOTSTRAP_CSS.relative_to(ROOT.parent)).replace("\\", "/"),
        },
        "cases": cases_out,
    }
    EXPECTED_MANIFEST.write_text(json.dumps(manifest, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")


def _verify_expected(records: list[dict]) -> list[str]:
    if not EXPECTED_MANIFEST.exists():
        return [f"Missing expected manifest: {EXPECTED_MANIFEST}"]
    expected = json.loads(EXPECTED_MANIFEST.read_text(encoding="utf-8"))
    expected_cases = {entry["case"]: entry for entry in expected.get("cases", [])}
    errors: list[str] = []

    for actual in records:
        case_name = actual["case"]
        exp = expected_cases.get(case_name)
        if exp is None:
            errors.append(f"[{case_name}] missing expected record")
            continue

        exp_pdf = exp["artifacts"]["pdf"]["sha256"]
        act_pdf = actual["artifacts"]["pdf"]["sha256"]
        if exp_pdf != act_pdf:
            errors.append(f"[{case_name}] pdf sha mismatch expected={exp_pdf} actual={act_pdf}")

        exp_pages = exp["artifacts"]["png_pages"]
        act_pages = actual["artifacts"]["png_pages"]
        if len(exp_pages) != len(act_pages):
            errors.append(
                f"[{case_name}] page count mismatch expected={len(exp_pages)} actual={len(act_pages)}"
            )
            continue
        for idx, (exp_page, act_page) in enumerate(zip(exp_pages, act_pages), start=1):
            if exp_page["sha256"] != act_page["sha256"]:
                errors.append(
                    f"[{case_name}] page {idx} sha mismatch expected={exp_page['sha256']} actual={act_page['sha256']}"
                )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate or verify Fullbleed public golden suite outputs.")
    parser.add_argument("mode", choices=["generate", "verify"], help="generate expected outputs or verify against them")
    parser.add_argument("--case", action="append", help="limit to one or more cases: invoice, statement, menu")
    parser.add_argument("--cli", help="override CLI command, e.g. 'fullbleed' or 'python -m fullbleed'")
    parser.add_argument("--image-dpi", type=int, default=144, help="PNG DPI for page artifact outputs")
    args = parser.parse_args()

    selected_cases = _select_cases(args.case)
    cli_cmd = _resolve_cli(args.cli)
    run_root = ROOT / "output" / args.mode
    run_root.mkdir(parents=True, exist_ok=True)

    for required in (BOOTSTRAP_CSS, NOTO_FONT):
        if not required.exists():
            raise FileNotFoundError(f"Required asset missing: {required}")

    records: list[dict] = []
    for case in selected_cases:
        print(f"[run] {case.name} via: {' '.join(cli_cmd)}")
        record = _run_render(case=case, cli_cmd=cli_cmd, run_root=run_root, image_dpi=args.image_dpi)
        records.append(record)
        print(
            f"[ok] {case.name} pdf_sha={record['artifacts']['pdf']['sha256']} "
            f"png_pages={len(record['artifacts']['png_pages'])}"
        )

    if args.mode == "generate":
        _write_expected(records=records, run_root=run_root)
        print(f"[ok] wrote expected manifest: {EXPECTED_MANIFEST}")
        return 0

    errors = _verify_expected(records=records)
    if errors:
        print("[fail] golden verification mismatch:")
        for err in errors:
            print(f"  - {err}")
        return 1
    print("[ok] all selected golden cases match expected hashes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

