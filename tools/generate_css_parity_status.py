#!/usr/bin/env python
"""Generate and validate CSS parity status artifacts from the ledger."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List


STATUS_ORDER = ["not_started", "in_progress", "partial", "implemented", "n/a"]
STAGE_ORDER = ["parser", "compute", "layout", "paint"]
PROGRESS_STATUSES = {"partial", "implemented"}


def _load_json(path: Path) -> Dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _percent(numerator: int, denominator: int) -> float:
    if denominator <= 0:
        return 0.0
    return round((numerator / denominator) * 100.0, 2)


def _validate_ledger(ledger: Dict[str, Any]) -> None:
    schema = ledger.get("schema")
    if schema != "fullbleed.css_parity_ledger.v1":
        raise ValueError(f"Unsupported ledger schema: {schema!r}")

    modules = ledger.get("modules")
    if not isinstance(modules, list) or not modules:
        raise ValueError("Ledger must contain a non-empty 'modules' list")

    seen_ids = set()
    for module in modules:
        module_id = module.get("id")
        if not module_id or not isinstance(module_id, str):
            raise ValueError("Each module must include a string 'id'")
        if module_id in seen_ids:
            raise ValueError(f"Duplicate module id: {module_id}")
        seen_ids.add(module_id)

        status = module.get("status")
        if status not in STATUS_ORDER:
            raise ValueError(f"Invalid status {status!r} for module {module_id}")

        stage_status = module.get("stage_status")
        if not isinstance(stage_status, dict):
            raise ValueError(f"Module {module_id} is missing stage_status")
        for stage in STAGE_ORDER:
            stage_value = stage_status.get(stage)
            if stage_value not in STATUS_ORDER:
                raise ValueError(
                    f"Invalid stage status {stage_value!r} for {module_id}.{stage}"
                )


def _stage_rollup(modules: List[Dict[str, Any]]) -> Dict[str, Dict[str, Any]]:
    rollup: Dict[str, Dict[str, Any]] = {}
    for stage in STAGE_ORDER:
        counts = {status: 0 for status in STATUS_ORDER}
        applicable_total = 0
        progress_total = 0
        implemented_total = 0
        for module in modules:
            status = module["stage_status"][stage]
            counts[status] += 1
            if status == "n/a":
                continue
            applicable_total += 1
            if status in PROGRESS_STATUSES:
                progress_total += 1
            if status == "implemented":
                implemented_total += 1

        rollup[stage] = {
            "counts": counts,
            "applicable_total": applicable_total,
            "progress_total": progress_total,
            "implemented_total": implemented_total,
            "progress_percent": _percent(progress_total, applicable_total),
            "implemented_percent": _percent(implemented_total, applicable_total),
        }
    return rollup


def _build_status_payload(ledger: Dict[str, Any], source_path: Path) -> Dict[str, Any]:
    modules = sorted(ledger["modules"], key=lambda item: item.get("priority", 10**9))

    module_counts = {status: 0 for status in STATUS_ORDER}
    for module in modules:
        module_counts[module["status"]] += 1

    total_modules = len(modules)
    module_progress = (
        module_counts["in_progress"]
        + module_counts["partial"]
        + module_counts["implemented"]
    )
    module_implemented = module_counts["implemented"]

    stage_rollup = _stage_rollup(modules)

    return {
        "schema": "fullbleed.css_parity_status.v1",
        "source_ledger": str(source_path).replace("\\", "/"),
        "summary": {
            "total_modules": total_modules,
            "module_counts": module_counts,
            "module_progress_total": module_progress,
            "module_implemented_total": module_implemented,
            "module_progress_percent": _percent(module_progress, total_modules),
            "module_implemented_percent": _percent(module_implemented, total_modules),
            "stage_rollup": stage_rollup,
        },
        "modules": modules,
    }


def _canonical_json(payload: Dict[str, Any]) -> str:
    return json.dumps(payload, ensure_ascii=True, indent=2, sort_keys=True) + "\n"


def run(input_path: Path, output_path: Path, check: bool, emit_json: bool) -> int:
    ledger = _load_json(input_path)
    _validate_ledger(ledger)
    payload = _build_status_payload(ledger, input_path)
    rendered = _canonical_json(payload)

    if check:
        if not output_path.exists():
            report = {
                "schema": "fullbleed.css_parity_status.check.v1",
                "ok": False,
                "reason": "missing_output",
                "output": str(output_path),
            }
            if emit_json:
                print(json.dumps(report, ensure_ascii=True))
            else:
                print(f"missing parity status output: {output_path}")
            return 1
        current = output_path.read_text(encoding="utf-8")
        ok = current == rendered
        report = {
            "schema": "fullbleed.css_parity_status.check.v1",
            "ok": ok,
            "output": str(output_path),
        }
        if emit_json:
            print(json.dumps(report, ensure_ascii=True))
        else:
            print(f"ok={ok} output={output_path}")
        return 0 if ok else 1

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(rendered, encoding="utf-8")
    report = {
        "schema": "fullbleed.css_parity_status.generate.v1",
        "ok": True,
        "output": str(output_path),
        "modules": payload["summary"]["total_modules"],
    }
    if emit_json:
        print(json.dumps(report, ensure_ascii=True))
    else:
        print(f"wrote {output_path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate or verify _css_working/css_parity_status.json from ledger data."
    )
    parser.add_argument(
        "--input",
        default="_css_working/css_parity_ledger.json",
        help="Path to the parity ledger JSON.",
    )
    parser.add_argument(
        "--out",
        default="_css_working/css_parity_status.json",
        help="Output path for generated status JSON.",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Check that --out matches generated content and exit non-zero on drift.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit command result as JSON.",
    )
    args = parser.parse_args()

    return run(Path(args.input), Path(args.out), args.check, args.json)


if __name__ == "__main__":
    raise SystemExit(main())
