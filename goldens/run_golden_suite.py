#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class GoldenCase:
    case_id: str
    report_path: Path
    output_dir: Path
    pdf_name: str
    png_glob: str
    png_name_re: str
    validation_name: str
    validation_kind: str
    css_layers_name: str | None = None


ROOT = Path(__file__).resolve().parents[1]
EXPECTED_DIR = ROOT / "goldens" / "expected"
EXPECTED_JSON = EXPECTED_DIR / "golden_suite.expected.json"
EXPECTED_PNG_ROOT = EXPECTED_DIR / "png"

CASES: tuple[GoldenCase, ...] = (
    GoldenCase(
        case_id="acme_invoice",
        report_path=ROOT / "examples" / "acme_invoice" / "report.py",
        output_dir=ROOT / "examples" / "acme_invoice" / "output",
        pdf_name="acme_sample_invoice.pdf",
        png_glob="acme_sample_invoice_page*.png",
        png_name_re=r"^acme_sample_invoice_page\d+\.png$",
        validation_name="acme_sample_invoice_component_mount_validation.json",
        validation_kind="component_mount",
        css_layers_name="acme_sample_invoice_css_layers.json",
    ),
    GoldenCase(
        case_id="bank_statement",
        report_path=ROOT / "examples" / "bank_statement" / "report.py",
        output_dir=ROOT / "examples" / "bank_statement" / "output",
        pdf_name="bank_statement.pdf",
        png_glob="bank_statement_page*.png",
        png_name_re=r"^bank_statement_page\d+\.png$",
        validation_name="bank_statement_component_mount_validation.json",
        validation_kind="component_mount",
        css_layers_name="bank_statement_css_layers.json",
    ),
    GoldenCase(
        case_id="coastal_menu",
        report_path=ROOT / "examples" / "coastal_menu" / "report.py",
        output_dir=ROOT / "examples" / "coastal_menu" / "output",
        pdf_name="coastal_menu.pdf",
        png_glob="coastal_menu_page*.png",
        png_name_re=r"^coastal_menu_page\d+\.png$",
        validation_name="coastal_menu_validation.json",
        validation_kind="coastal_validation",
    ),
)


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _assert_component_mount_contract(payload: dict[str, Any], *, case_id: str) -> list[str]:
    failures: list[str] = []
    if not payload.get("ok", False):
        failures.append(f"{case_id}: component mount validation not ok")
    if int(payload.get("missing_glyph_count", 0)) != 0:
        failures.append(f"{case_id}: missing_glyph_count != 0")
    if int(payload.get("overflow_count", 0)) != 0:
        failures.append(f"{case_id}: overflow_count != 0")
    if int(payload.get("css_warning_count", 0)) != 0:
        failures.append(f"{case_id}: css_warning_count != 0")
    if int(payload.get("known_loss_count", 0)) != 0:
        failures.append(f"{case_id}: known_loss_count != 0")
    if int(payload.get("html_asset_warning_count", 0)) != 0:
        failures.append(f"{case_id}: html_asset_warning_count != 0")
    if payload.get("failures"):
        failures.append(f"{case_id}: validation payload has failures entries")
    if payload.get("warnings"):
        failures.append(f"{case_id}: validation payload has warnings entries")
    return failures


def _assert_coastal_contract(payload: dict[str, Any], *, case_id: str) -> list[str]:
    failures: list[str] = []
    if not payload.get("ok", False):
        failures.append(f"{case_id}: coastal validation not ok")
    checks = payload.get("checks") or {}
    if int(checks.get("missing_glyph_count", 0)) != 0:
        failures.append(f"{case_id}: checks.missing_glyph_count != 0")
    diagnostics = payload.get("diagnostics") or []
    if diagnostics:
        failures.append(f"{case_id}: diagnostics not empty")
    return failures


def _assert_css_contract(payload: dict[str, Any], *, case_id: str) -> list[str]:
    failures: list[str] = []
    if int(payload.get("unscoped_selector_count", -1)) != 0:
        failures.append(f"{case_id}: unscoped_selector_count != 0")
    if int(payload.get("no_effect_declaration_count", -1)) != 0:
        failures.append(f"{case_id}: no_effect_declaration_count != 0")
    for layer in payload.get("layers", []):
        if not layer.get("exists", False):
            failures.append(f"{case_id}: missing css layer {layer.get('path')}")
    return failures


def _run_case(case: GoldenCase, *, python_exec: str) -> dict[str, Any]:
    env = os.environ.copy()
    pythonpath_parts = [str(ROOT / "python")]
    if env.get("PYTHONPATH"):
        pythonpath_parts.append(env["PYTHONPATH"])
    env["PYTHONPATH"] = os.pathsep.join(pythonpath_parts)

    proc = subprocess.run(
        [python_exec, str(case.report_path)],
        cwd=str(ROOT),
        env=env,
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"{case.case_id}: render failed ({proc.returncode})\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )

    pdf_path = case.output_dir / case.pdf_name
    if not pdf_path.exists():
        raise FileNotFoundError(f"{case.case_id}: missing PDF output {pdf_path}")

    png_paths = sorted(
        path
        for path in case.output_dir.glob(case.png_glob)
        if re.match(case.png_name_re, path.name)
    )
    if not png_paths:
        raise FileNotFoundError(f"{case.case_id}: no PNG outputs matched {case.png_glob}")

    validation_path = case.output_dir / case.validation_name
    if not validation_path.exists():
        raise FileNotFoundError(f"{case.case_id}: missing validation file {validation_path}")
    validation_payload = json.loads(validation_path.read_text(encoding="utf-8"))

    failures: list[str] = []
    if case.validation_kind == "component_mount":
        failures.extend(_assert_component_mount_contract(validation_payload, case_id=case.case_id))
    elif case.validation_kind == "coastal_validation":
        failures.extend(_assert_coastal_contract(validation_payload, case_id=case.case_id))
    else:
        failures.append(f"{case.case_id}: unknown validation kind {case.validation_kind!r}")

    css_payload: dict[str, Any] | None = None
    if case.css_layers_name:
        css_path = case.output_dir / case.css_layers_name
        if not css_path.exists():
            failures.append(f"{case.case_id}: missing css layers file {css_path}")
        else:
            css_payload = json.loads(css_path.read_text(encoding="utf-8"))
            failures.extend(_assert_css_contract(css_payload, case_id=case.case_id))

    if failures:
        joined = "\n".join(f"- {item}" for item in failures)
        raise RuntimeError(f"{case.case_id}: contract failures:\n{joined}")

    png_hashes = {path.name: _sha256(path) for path in png_paths}
    result = {
        "case_id": case.case_id,
        "report": str(case.report_path.relative_to(ROOT)).replace("\\", "/"),
        "pdf": {
            "path": str(pdf_path.relative_to(ROOT)).replace("\\", "/"),
            "bytes": pdf_path.stat().st_size,
            "sha256": _sha256(pdf_path),
        },
        "png": {
            "count": len(png_paths),
            "files": [
                {
                    "name": path.name,
                    "path": str(path.relative_to(ROOT)).replace("\\", "/"),
                    "bytes": path.stat().st_size,
                    "sha256": png_hashes[path.name],
                }
                for path in png_paths
            ],
        },
        "validation": {
            "path": str(validation_path.relative_to(ROOT)).replace("\\", "/"),
            "kind": case.validation_kind,
            "sha256": _sha256(validation_path),
        },
        "css_layers": None
        if case.css_layers_name is None
        else {
            "path": str((case.output_dir / case.css_layers_name).relative_to(ROOT)).replace("\\", "/"),
            "sha256": _sha256(case.output_dir / case.css_layers_name),
        },
    }
    return result


def _copy_png_baselines(case_id: str, png_entries: list[dict[str, Any]]) -> None:
    case_dir = EXPECTED_PNG_ROOT / case_id
    if case_dir.exists():
        shutil.rmtree(case_dir)
    case_dir.mkdir(parents=True, exist_ok=True)
    for entry in png_entries:
        src = ROOT / entry["path"]
        dst = case_dir / entry["name"]
        shutil.copyfile(src, dst)


def _selected_cases(case_ids: list[str] | None) -> list[GoldenCase]:
    if not case_ids:
        return list(CASES)
    wanted = {item.strip() for item in case_ids if item.strip()}
    by_id = {case.case_id: case for case in CASES}
    missing = sorted(wanted - set(by_id.keys()))
    if missing:
        raise ValueError(f"Unknown case id(s): {', '.join(missing)}")
    return [by_id[case_id] for case_id in sorted(wanted)]


def _load_expected() -> dict[str, Any]:
    if not EXPECTED_JSON.exists():
        raise FileNotFoundError(
            f"Missing expected manifest: {EXPECTED_JSON}. Run generate first."
        )
    return json.loads(EXPECTED_JSON.read_text(encoding="utf-8"))


def cmd_generate(*, python_exec: str, case_ids: list[str] | None) -> int:
    selected = _selected_cases(case_ids)
    suite_results: dict[str, Any] = {}
    for case in selected:
        suite_results[case.case_id] = _run_case(case, python_exec=python_exec)
    EXPECTED_DIR.mkdir(parents=True, exist_ok=True)
    EXPECTED_PNG_ROOT.mkdir(parents=True, exist_ok=True)
    for case_id, result in suite_results.items():
        _copy_png_baselines(case_id, result["png"]["files"])

    manifest = {
        "schema": "fullbleed.golden_suite.v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "cases": suite_results,
    }
    EXPECTED_JSON.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"[ok] Wrote expected manifest: {EXPECTED_JSON}")
    print(f"[ok] Wrote PNG baselines: {EXPECTED_PNG_ROOT}")
    print(f"[ok] Cases: {', '.join(sorted(suite_results.keys()))}")
    return 0


def _compare_case(expected_case: dict[str, Any], actual_case: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    case_id = actual_case["case_id"]
    if expected_case["pdf"]["sha256"] != actual_case["pdf"]["sha256"]:
        failures.append(
            f"{case_id}: PDF hash mismatch expected={expected_case['pdf']['sha256']} actual={actual_case['pdf']['sha256']}"
        )
    expected_png = {item["name"]: item["sha256"] for item in expected_case["png"]["files"]}
    actual_png = {item["name"]: item["sha256"] for item in actual_case["png"]["files"]}
    if set(expected_png.keys()) != set(actual_png.keys()):
        failures.append(
            f"{case_id}: PNG page set mismatch expected={sorted(expected_png.keys())} actual={sorted(actual_png.keys())}"
        )
    for name, expected_hash in expected_png.items():
        actual_hash = actual_png.get(name)
        if actual_hash != expected_hash:
            failures.append(
                f"{case_id}: PNG hash mismatch for {name} expected={expected_hash} actual={actual_hash}"
            )
    baseline_dir = EXPECTED_PNG_ROOT / case_id
    for name, expected_hash in expected_png.items():
        baseline_path = baseline_dir / name
        if not baseline_path.exists():
            failures.append(f"{case_id}: missing baseline file {baseline_path}")
            continue
        baseline_hash = _sha256(baseline_path)
        if baseline_hash != expected_hash:
            failures.append(
                f"{case_id}: baseline hash drift for {name} expected={expected_hash} baseline={baseline_hash}"
            )
    return failures


def cmd_verify(*, python_exec: str, case_ids: list[str] | None) -> int:
    expected = _load_expected()
    expected_cases = expected.get("cases", {})
    selected = _selected_cases(case_ids)
    failures: list[str] = []
    for case in selected:
        if case.case_id not in expected_cases:
            failures.append(f"{case.case_id}: missing from expected manifest")
            continue
        actual_case = _run_case(case, python_exec=python_exec)
        failures.extend(_compare_case(expected_cases[case.case_id], actual_case))
    if failures:
        print("[error] Golden verification failed:")
        for item in failures:
            print(f"  - {item}")
        return 1
    print(f"[ok] Golden verification passed for {len(selected)} case(s)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Run component-scaffolded golden suite.")
    parser.add_argument(
        "--python",
        default=sys.executable,
        help="Python executable used to run each example report.py",
    )
    parser.add_argument(
        "--cases",
        default="",
        help="Comma-separated subset of case ids (default: all)",
    )
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("generate", help="Render cases and refresh expected manifest/baselines")
    sub.add_parser("verify", help="Render cases and compare to expected manifest/baselines")
    args = parser.parse_args()

    case_ids = [part.strip() for part in args.cases.split(",") if part.strip()]
    if args.command == "generate":
        return cmd_generate(python_exec=args.python, case_ids=case_ids)
    if args.command == "verify":
        return cmd_verify(python_exec=args.python, case_ids=case_ids)
    raise RuntimeError(f"Unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
