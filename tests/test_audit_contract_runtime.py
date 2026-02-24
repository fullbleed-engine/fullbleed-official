from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

import fullbleed
from fullbleed.audit_section508 import (
    load_section508_html_registry,
    section508_html_coverage_from_findings,
)
from fullbleed.audit_wcag import load_wcag20aa_registry, wcag20aa_coverage_from_findings


ROOT = Path(__file__).resolve().parents[1]
SPECS = ROOT / "docs" / "specs"


def _require_contract_api() -> None:
    if not hasattr(fullbleed, "audit_contract_metadata") or not hasattr(
        fullbleed, "audit_contract_registry"
    ):
        pytest.skip("fullbleed native extension contract API is not available")
    if not hasattr(fullbleed, "audit_contract_wcag20aa_coverage"):
        pytest.skip("fullbleed native WCAG coverage helper is not available")
    if not hasattr(fullbleed, "audit_contract_section508_html_coverage"):
        pytest.skip("fullbleed native Section 508 HTML coverage helper is not available")


def _sha(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def test_audit_contract_runtime_metadata_is_stable() -> None:
    _require_contract_api()
    m1 = fullbleed.audit_contract_metadata()
    m2 = fullbleed.audit_contract_metadata()

    assert m1 == m2
    assert m1["contract_id"] == "fullbleed.audit_contract"
    assert m1["contract_version"] == "1"
    assert m1["contract_fingerprint"].startswith("sha256:")
    assert len(m1["registries"]) >= 2


def test_audit_contract_runtime_registries_match_spec_artifacts() -> None:
    _require_contract_api()

    embedded_audit = fullbleed.audit_contract_registry("fullbleed.audit_registry.v1")
    embedded_wcag = fullbleed.audit_contract_registry("wcag20aa_registry.v1")
    embedded_s508 = fullbleed.audit_contract_registry("section508_html_registry.v1")

    spec_audit = (SPECS / "fullbleed.audit_registry.v1.yaml").read_bytes().decode("utf-8")
    spec_wcag = (SPECS / "wcag20aa_registry.v1.yaml").read_bytes().decode("utf-8")
    spec_s508 = (SPECS / "section508_html_registry.v1.yaml").read_bytes().decode("utf-8")

    assert embedded_audit == spec_audit
    assert embedded_wcag == spec_wcag
    assert embedded_s508 == spec_s508
    assert json.loads(embedded_audit) == json.loads(spec_audit)
    assert json.loads(embedded_wcag) == json.loads(spec_wcag)
    assert json.loads(embedded_s508) == json.loads(spec_s508)

    meta = fullbleed.audit_contract_metadata()
    reg_hashes = {row["id"]: row["hash"] for row in meta["registries"]}
    assert reg_hashes["fullbleed.audit_registry.v1"] == _sha(embedded_audit)
    assert reg_hashes["wcag20aa_registry.v1"] == _sha(embedded_wcag)
    assert reg_hashes["section508_html_registry.v1"] == _sha(embedded_s508)

    # Contract fingerprint is a build-level aggregate, so just assert format + stability here.
    assert isinstance(meta["contract_fingerprint"], str)
    assert meta["contract_fingerprint"].startswith("sha256:")


def test_audit_contract_registry_unknown_name_raises() -> None:
    _require_contract_api()
    with pytest.raises(ValueError):
        fullbleed.audit_contract_registry("unknown.registry")


def test_audit_contract_wcag20aa_coverage_matches_python_fallback() -> None:
    _require_contract_api()
    findings = [
        {"rule_id": "fb.a11y.html.lang_present_valid", "verdict": "pass"},
        {"rule_id": "fb.a11y.html.title_present_nonempty", "verdict": "pass"},
        {"rule_id": "fb.a11y.ids.duplicate_id", "verdict": "fail"},
        {"rule_id": "fb.a11y.signatures.text_semantics_present", "verdict": "manual_needed"},
        {"rule_id": "fb.a11y.aria.reference_target_exists", "verdict": "pass"},
    ]
    native = fullbleed.audit_contract_wcag20aa_coverage(findings)
    py_fallback = wcag20aa_coverage_from_findings(findings, registry=load_wcag20aa_registry())
    assert native == py_fallback


def test_audit_contract_section508_html_coverage_matches_python_fallback() -> None:
    _require_contract_api()
    findings = [
        {"rule_id": "fb.a11y.html.lang_present_valid", "verdict": "pass"},
        {"rule_id": "fb.a11y.html.title_present_nonempty", "verdict": "pass"},
        {"rule_id": "fb.a11y.claim.wcag20aa_level_readiness", "verdict": "warn"},
    ]
    native = fullbleed.audit_contract_section508_html_coverage(findings)
    py_fallback = section508_html_coverage_from_findings(
        findings, registry=load_section508_html_registry()
    )
    assert native == py_fallback
