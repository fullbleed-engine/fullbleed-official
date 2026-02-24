from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any

from .audit_wcag import load_wcag20aa_registry, wcag20aa_coverage_from_findings


def _repo_root_from_package() -> Path:
    # python/fullbleed/audit_section508.py -> repo root is parents[2]
    return Path(__file__).resolve().parents[2]


def _section508_registry_path() -> Path:
    return _repo_root_from_package() / "docs" / "specs" / "section508_html_registry.v1.yaml"


def _runtime_registry_json_from_engine(name: str) -> str | None:
    try:
        import fullbleed

        fn = getattr(fullbleed, "audit_contract_registry", None)
        if not callable(fn):
            return None
        value = fn(name)
        if isinstance(value, str) and value.strip():
            return value
    except Exception:
        return None
    return None


def _runtime_section508_coverage_from_engine(
    findings: list[dict[str, Any]],
) -> dict[str, Any] | None:
    try:
        import fullbleed

        fn = getattr(fullbleed, "audit_contract_section508_html_coverage", None)
        if not callable(fn):
            return None
        value = fn(findings)
        if isinstance(value, dict):
            return value
    except Exception:
        return None
    return None


@lru_cache(maxsize=1)
def load_section508_html_registry() -> dict[str, Any]:
    text = _runtime_registry_json_from_engine("section508_html_registry.v1")
    if text:
        return json.loads(text)
    return json.loads(_section508_registry_path().read_text(encoding="utf-8"))


def _worst_verdict(verdicts: list[str]) -> str | None:
    if not verdicts:
        return None
    order = {"fail": 5, "warn": 4, "manual_needed": 3, "pass": 2, "not_applicable": 1}
    return max(verdicts, key=lambda v: order.get(v, 0))


def section508_html_coverage_from_findings(
    findings: list[dict[str, Any]], *, registry: dict[str, Any] | None = None
) -> dict[str, Any]:
    if registry is None:
        native = _runtime_section508_coverage_from_engine(findings)
        if native is not None:
            return native

    reg = registry or load_section508_html_registry()
    wcag_cov = wcag20aa_coverage_from_findings(findings, registry=load_wcag20aa_registry())
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
    implemented_entries: list[dict[str, Any]] = []
    supporting_only_entries: list[dict[str, Any]] = []
    planned_only_entries: list[dict[str, Any]] = []
    for entry in mapped_entries:
        statuses = {str(m.get("status")) for m in _entry_mappings(entry)}
        if "implemented" in statuses:
            implemented_entries.append(entry)
        elif statuses == {"supporting"} or ("supporting" in statuses and "planned" not in statuses):
            supporting_only_entries.append(entry)
        else:
            planned_only_entries.append(entry)

    specific_impl_eval = 0
    specific_impl_pending = 0
    specific_result_counts = {
        "pass": 0,
        "fail": 0,
        "warn": 0,
        "manual_needed": 0,
        "not_applicable": 0,
        "unknown": 0,
    }
    for entry in implemented_entries:
        verdicts: list[str] = []
        for mapping in _entry_mappings(entry):
            if str(mapping.get("status")) != "implemented":
                continue
            verdicts.extend(rule_verdicts.get(str(mapping.get("id") or ""), []))
        if not verdicts:
            specific_impl_pending += 1
            continue
        specific_impl_eval += 1
        worst = _worst_verdict(verdicts) or "unknown"
        specific_result_counts[worst if worst in specific_result_counts else "unknown"] += 1

    scope = reg.get("scope", {})
    specific_total = int(scope.get("total_specific_entries", len(entries)))
    inherited_wcag_total = int(scope.get("inherited_wcag_entry_count", wcag_cov.get("total_entries", 0)))
    total_entries = int(scope.get("total_entries", specific_total + inherited_wcag_total))

    combined_counts = dict(wcag_cov.get("implemented_mapped_result_counts", {}))
    for key, val in specific_result_counts.items():
        combined_counts[key] = int(combined_counts.get(key, 0)) + int(val)

    specific_mapped = len(mapped_entries)
    specific_impl = len(implemented_entries)
    mapped_entry_count = int(wcag_cov.get("mapped_entry_count", 0)) + specific_mapped
    implemented_mapped_entry_count = int(wcag_cov.get("implemented_mapped_entry_count", 0)) + specific_impl
    implemented_mapped_entry_evaluated_count = (
        int(wcag_cov.get("implemented_mapped_entry_evaluated_count", 0)) + specific_impl_eval
    )
    implemented_mapped_entry_pending_count = (
        int(wcag_cov.get("implemented_mapped_entry_pending_count", 0)) + specific_impl_pending
    )

    return {
        "registry_id": str(reg.get("schema") or "section508_html_registry.v1"),
        "registry_version": int(reg.get("version", 1)),
        "profile_id": str(reg.get("profile_id") or "section508.revised.e205.html"),
        "total_entries": total_entries,
        "specific_entries_total": specific_total,
        "inherited_wcag_entries_total": inherited_wcag_total,
        "mapped_entry_count": mapped_entry_count,
        "implemented_mapped_entry_count": implemented_mapped_entry_count,
        "implemented_mapped_entry_evaluated_count": implemented_mapped_entry_evaluated_count,
        "implemented_mapped_entry_pending_count": implemented_mapped_entry_pending_count,
        "supporting_only_mapped_entry_count": int(wcag_cov.get("supporting_only_mapped_entry_count", 0))
        + len(supporting_only_entries),
        "planned_only_mapped_entry_count": int(wcag_cov.get("planned_only_mapped_entry_count", 0))
        + len(planned_only_entries),
        "unmapped_entry_count": max(0, total_entries - mapped_entry_count),
        "specific_mapped_entry_count": specific_mapped,
        "specific_implemented_mapped_entry_count": specific_impl,
        "specific_implemented_mapped_entry_evaluated_count": specific_impl_eval,
        "specific_implemented_mapped_entry_pending_count": specific_impl_pending,
        "specific_unmapped_entry_count": max(0, specific_total - specific_mapped),
        "inherited_wcag_registry_id": str(wcag_cov.get("registry_id") or "wcag20aa_registry.v1"),
        "inherited_wcag_implemented_mapped_entry_count": int(
            wcag_cov.get("implemented_mapped_entry_count", 0)
        ),
        "inherited_wcag_implemented_mapped_entry_evaluated_count": int(
            wcag_cov.get("implemented_mapped_entry_evaluated_count", 0)
        ),
        "inherited_wcag_unmapped_entry_count": int(wcag_cov.get("unmapped_entry_count", 0)),
        "implemented_mapped_result_counts": combined_counts,
    }
