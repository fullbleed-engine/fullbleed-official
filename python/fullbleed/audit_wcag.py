from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any


def _repo_root_from_package() -> Path:
    # python/fullbleed/audit_wcag.py -> repo root is parents[2]
    return Path(__file__).resolve().parents[2]


def _wcag_registry_path() -> Path:
    return _repo_root_from_package() / "docs" / "specs" / "wcag20aa_registry.v1.yaml"


def _runtime_registry_json_from_engine(name: str) -> str | None:
    try:
        import fullbleed  # local package; may or may not have native extension helpers

        fn = getattr(fullbleed, "audit_contract_registry", None)
        if not callable(fn):
            return None
        value = fn(name)
        if isinstance(value, str) and value.strip():
            return value
    except Exception:
        return None
    return None


def _runtime_wcag20aa_coverage_from_engine(
    findings: list[dict[str, Any]],
) -> dict[str, Any] | None:
    try:
        import fullbleed  # local package; may or may not have native extension helpers

        fn = getattr(fullbleed, "audit_contract_wcag20aa_coverage", None)
        if not callable(fn):
            return None
        value = fn(findings)
        if isinstance(value, dict):
            return value
    except Exception:
        return None
    return None


@lru_cache(maxsize=1)
def load_wcag20aa_registry() -> dict[str, Any]:
    text = _runtime_registry_json_from_engine("wcag20aa_registry.v1")
    if text:
        return json.loads(text)
    path = _wcag_registry_path()
    return json.loads(path.read_text(encoding="utf-8"))


def _worst_verdict(verdicts: list[str]) -> str | None:
    if not verdicts:
        return None
    order = {"fail": 5, "warn": 4, "manual_needed": 3, "pass": 2, "not_applicable": 1}
    return max(verdicts, key=lambda v: order.get(v, 0))


def wcag20aa_coverage_from_findings(
    findings: list[dict[str, Any]], *, registry: dict[str, Any] | None = None
) -> dict[str, Any]:
    if registry is None:
        native = _runtime_wcag20aa_coverage_from_engine(findings)
        if native is not None:
            return native

    reg = registry or load_wcag20aa_registry()
    entries = list(reg.get("entries", []))

    rule_verdicts: dict[str, list[str]] = {}
    for finding in findings:
        rid = str(finding.get("rule_id") or "").strip()
        if not rid:
            continue
        rule_verdicts.setdefault(rid, []).append(str(finding.get("verdict") or ""))

    def _entry_mappings(entry: dict[str, Any]) -> list[dict[str, Any]]:
        return [m for m in entry.get("fullbleed_rule_mapping", []) if isinstance(m, dict)]

    mapped_entries = [e for e in entries if _entry_mappings(e)]
    sc_entries = [e for e in entries if e.get("kind") == "success_criterion"]
    conf_entries = [e for e in entries if e.get("kind") == "conformance_requirement"]

    implemented_entries: list[dict[str, Any]] = []
    supporting_only_entries: list[dict[str, Any]] = []
    planned_only_entries: list[dict[str, Any]] = []
    for entry in mapped_entries:
        maps = _entry_mappings(entry)
        statuses = {str(m.get("status")) for m in maps}
        if "implemented" in statuses:
            implemented_entries.append(entry)
        elif statuses == {"supporting"} or ("supporting" in statuses and "planned" not in statuses):
            supporting_only_entries.append(entry)
        else:
            planned_only_entries.append(entry)

    implemented_evaluated = 0
    implemented_pending = 0
    implemented_result_counts = {
        "pass": 0,
        "fail": 0,
        "warn": 0,
        "manual_needed": 0,
        "not_applicable": 0,
        "unknown": 0,
    }
    for entry in implemented_entries:
        rule_ids = [
            str(m.get("id"))
            for m in _entry_mappings(entry)
            if str(m.get("status")) == "implemented"
        ]
        verdicts: list[str] = []
        for rid in rule_ids:
            verdicts.extend(rule_verdicts.get(rid, []))
        if verdicts:
            implemented_evaluated += 1
            worst = _worst_verdict(verdicts) or "unknown"
            implemented_result_counts[worst if worst in implemented_result_counts else "unknown"] += 1
        else:
            implemented_pending += 1

    mapped_sc_count = sum(1 for e in sc_entries if _entry_mappings(e))
    mapped_conf_count = sum(1 for e in conf_entries if _entry_mappings(e))

    total_entries = int(reg.get("scope", {}).get("total_entries", len(entries)))
    total_sc = int(reg.get("scope", {}).get("total_success_criteria", len(sc_entries)))
    total_conf = int(reg.get("scope", {}).get("total_conformance_requirements", len(conf_entries)))

    return {
        "registry_id": str(reg.get("schema") or "wcag20aa_registry.v1"),
        "registry_version": int(reg.get("version", 1)),
        "wcag_version": str(reg.get("wcag_version") or "2.0"),
        "target_level": str(reg.get("target_level") or "AA"),
        "total_entries": total_entries,
        "success_criteria_total": total_sc,
        "conformance_requirements_total": total_conf,
        "mapped_entry_count": len(mapped_entries),
        "mapped_success_criteria_count": mapped_sc_count,
        "mapped_conformance_requirement_count": mapped_conf_count,
        "implemented_mapped_entry_count": len(implemented_entries),
        "implemented_mapped_entry_evaluated_count": implemented_evaluated,
        "implemented_mapped_entry_pending_count": implemented_pending,
        "supporting_only_mapped_entry_count": len(supporting_only_entries),
        "planned_only_mapped_entry_count": len(planned_only_entries),
        "unmapped_entry_count": max(0, total_entries - len(mapped_entries)),
        "implemented_mapped_result_counts": implemented_result_counts,
    }
