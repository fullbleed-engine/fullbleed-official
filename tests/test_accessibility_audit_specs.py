from __future__ import annotations

import json
from pathlib import Path

import pytest
from fullbleed.audit_wcag import wcag20aa_coverage_from_findings
from fullbleed.audit_section508 import section508_html_coverage_from_findings


ROOT = Path(__file__).resolve().parents[1]
SPECS = ROOT / "docs" / "specs"
EXAMPLES = SPECS / "examples"


def _load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _load_registry_yaml_as_json(path: Path) -> dict:
    # The registry is stored as JSON-formatted YAML (valid YAML) so we can validate it
    # with stdlib `json` without introducing an additional parser dependency.
    return _load_json(path)


@pytest.fixture(scope="module")
def jsonschema_module():
    return pytest.importorskip("jsonschema")


def test_claim_language_policy_contains_required_guardrail() -> None:
    text = (SPECS / "accessibility-claim-language.md").read_text(encoding="utf-8")
    assert "not legal conformance certification" in text
    assert "Paged Media Rank is an operational compatibility score" in text


def test_schemas_validate_example_fixtures(jsonschema_module) -> None:
    validator_cls = jsonschema_module.Draft202012Validator

    verifier_schema = _load_json(SPECS / "fullbleed.a11y.verify.v1.schema.json")
    pmr_schema = _load_json(SPECS / "fullbleed.pmr.v1.schema.json")
    verifier_example = _load_json(EXAMPLES / "fullbleed.a11y.verify.v1.example.json")
    pmr_example = _load_json(EXAMPLES / "fullbleed.pmr.v1.example.json")

    validator_cls.check_schema(verifier_schema)
    validator_cls.check_schema(pmr_schema)
    validator_cls(verifier_schema).validate(verifier_example)
    validator_cls(pmr_schema).validate(pmr_example)


def test_registry_is_consistent_with_profiles_and_categories() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "fullbleed.audit_registry.v1.yaml")

    assert registry["schema"] == "fullbleed.audit_registry.v1"
    entries = list(registry["entries"])
    entry_ids = [entry["id"] for entry in entries]
    assert len(entry_ids) == len(set(entry_ids)), "registry entry IDs must be unique"

    categories = list(registry["pmr_categories"])
    category_ids = [c["id"] for c in categories]
    assert len(category_ids) == len(set(category_ids)), "PMR categories must be unique"
    assert sum(float(c["weight"]) for c in categories) == pytest.approx(100.0)

    category_id_set = set(category_ids)
    entry_id_set = set(entry_ids)

    for entry in entries:
        if entry["system"] == "pmr":
            assert entry["category"] in category_id_set
            if entry.get("scored", False):
                assert float(entry.get("weight", 0)) > 0
        if entry["kind"] == "rule":
            assert "category" not in entry

    for profile_name, profile in registry["profiles"].items():
        assert profile["default_mode"] in {"off", "warn", "error"}, profile_name
        for override in profile.get("overrides", []):
            assert override["level"] in {"off", "warn", "error"}, override
            assert override["id"] in entry_id_set, override


def test_examples_reference_known_registry_entries() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "fullbleed.audit_registry.v1.yaml")
    known_ids = {entry["id"] for entry in registry["entries"]}

    verifier_example = _load_json(EXAMPLES / "fullbleed.a11y.verify.v1.example.json")
    pmr_example = _load_json(EXAMPLES / "fullbleed.pmr.v1.example.json")

    for finding in verifier_example["findings"]:
        # Manual-only placeholder items are allowed to exist outside the seeded registry in v1.
        if finding["verification_mode"] == "manual":
            continue
        assert finding["rule_id"] in known_ids, finding["rule_id"]

    pmr_category_ids = {c["id"] for c in registry["pmr_categories"]}
    pmr_weight_sum = sum(float(c["weight"]) for c in pmr_example["categories"])
    assert pmr_weight_sum == pytest.approx(100.0)

    for cat in pmr_example["categories"]:
        assert cat["id"] in pmr_category_ids

    for audit in pmr_example["audits"]:
        assert audit["audit_id"] in known_ids, audit["audit_id"]
        assert audit["category"] in pmr_category_ids, audit["category"]


def test_wcag20aa_registry_enumerates_all_a_aa_sc_and_conformance_requirements() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "wcag20aa_registry.v1.yaml")

    assert registry["schema"] == "wcag20aa_registry.v1"
    scope = registry["scope"]
    assert scope["includes_success_criteria_levels"] == ["A", "AA"]
    assert scope["includes_conformance_requirements"] is True

    entries = list(registry["entries"])
    ids = [e["id"] for e in entries]
    assert len(ids) == len(set(ids)), "WCAG registry entry IDs must be unique"

    sc_entries = [e for e in entries if e["kind"] == "success_criterion"]
    conf_entries = [e for e in entries if e["kind"] == "conformance_requirement"]
    assert len(sc_entries) == 38
    assert len(conf_entries) == 5
    assert scope["total_success_criteria"] == 38
    assert scope["total_conformance_requirements"] == 5
    assert scope["total_entries"] == 43

    levels = {e["level"] for e in sc_entries}
    assert levels <= {"A", "AA"}

    principle_counts = {}
    for entry in sc_entries:
        principle_counts[entry["principle"]] = principle_counts.get(entry["principle"], 0) + 1
        assert entry["applicability"] in {"always", "conditional"}
        assert entry["verification_mode"] in {"machine", "hybrid", "manual"}
        assert entry["default_gate_level"] in {"off", "warn", "error"}
        assert isinstance(entry["evidence_requirements"], list) and entry["evidence_requirements"]
        assert isinstance(entry["fullbleed_rule_mapping"], list)

    assert principle_counts == {
        "Perceivable": 14,
        "Operable": 12,
        "Understandable": 10,
        "Robust": 2,
    }

    evidence_catalog = registry["evidence_requirement_catalog"]
    for entry in entries:
        assert isinstance(entry["evidence_requirements"], list) and entry["evidence_requirements"]
        for ev_id in entry["evidence_requirements"]:
            assert ev_id in evidence_catalog, (entry["id"], ev_id)
        for mapping in entry.get("fullbleed_rule_mapping", []):
            assert mapping["system"] in {"a11y_verifier", "pmr"}
            assert mapping["status"] in {"implemented", "planned", "supporting"}
            assert mapping["coverage"] in {"partial", "supporting"}

    required_ids = {
        "wcag20.sc.1.1.1",
        "wcag20.sc.1.3.1",
        "wcag20.sc.2.4.2",
        "wcag20.sc.3.1.1",
        "wcag20.sc.4.1.1",
        "wcag20.sc.4.1.2",
        "wcag20.conf.level",
        "wcag20.conf.full_pages",
        "wcag20.conf.complete_processes",
        "wcag20.conf.accessibility_supported_technologies",
        "wcag20.conf.non_interference",
    }
    assert required_ids <= set(ids)


def test_wcag20aa_runtime_helper_matches_registry_counts() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "wcag20aa_registry.v1.yaml")
    findings = [
        {"rule_id": "fb.a11y.html.lang_present_valid", "verdict": "pass"},
        {"rule_id": "fb.a11y.html.title_present_nonempty", "verdict": "pass"},
        {"rule_id": "fb.a11y.structure.single_main", "verdict": "pass"},
        {"rule_id": "fb.a11y.ids.duplicate_id", "verdict": "pass"},
        {"rule_id": "fb.a11y.aria.reference_target_exists", "verdict": "pass"},
        {"rule_id": "fb.a11y.signatures.text_semantics_present", "verdict": "pass"},
    ]
    summary = wcag20aa_coverage_from_findings(findings, registry=registry)

    assert summary["registry_id"] == "wcag20aa_registry.v1"
    assert summary["total_entries"] == 43
    assert summary["success_criteria_total"] == 38
    assert summary["conformance_requirements_total"] == 5
    assert summary["mapped_entry_count"] + summary["unmapped_entry_count"] == 43
    assert (
        summary["implemented_mapped_entry_evaluated_count"]
        <= summary["implemented_mapped_entry_count"]
    )


def test_section508_html_registry_scope_and_mappings_are_well_formed() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "section508_html_registry.v1.yaml")

    assert registry["schema"] == "section508_html_registry.v1"
    scope = registry["scope"]
    assert scope["includes_wcag20aa_incorporation"] is True
    assert scope["inherited_wcag_registry_id"] == "wcag20aa_registry.v1"
    assert scope["inherited_wcag_entry_count"] == 43
    assert scope["total_specific_entries"] == 6
    assert scope["total_entries"] == 49

    entries = list(registry["entries"])
    ids = [e["id"] for e in entries]
    assert len(ids) == len(set(ids)), "Section 508 registry entry IDs must be unique"
    assert len(entries) == 6

    evidence_catalog = registry["evidence_requirement_catalog"]
    for entry in entries:
        assert entry["verification_mode"] in {"machine", "hybrid", "manual"}
        assert entry["applicability"] in {"always", "conditional"}
        assert entry["default_gate_level"] in {"off", "warn", "error"}
        for ev_id in entry["evidence_requirements"]:
            assert ev_id in evidence_catalog, (entry["id"], ev_id)
        for mapping in entry.get("fullbleed_rule_mapping", []):
            assert mapping["system"] in {"a11y_verifier", "pmr"}
            assert mapping["status"] in {"implemented", "planned", "supporting"}
            assert mapping["coverage"] in {"partial", "supporting"}

    assert "s508.e205.4.wcag20aa_incorporation" in ids


def test_section508_html_runtime_helper_composes_with_wcag_inherited_coverage() -> None:
    registry = _load_registry_yaml_as_json(SPECS / "section508_html_registry.v1.yaml")
    findings = [
        {"rule_id": "fb.a11y.html.lang_present_valid", "verdict": "pass"},
        {"rule_id": "fb.a11y.html.title_present_nonempty", "verdict": "pass"},
        {"rule_id": "fb.a11y.claim.wcag20aa_level_readiness", "verdict": "warn"},
    ]
    summary = section508_html_coverage_from_findings(findings, registry=registry)

    assert summary["registry_id"] == "section508_html_registry.v1"
    assert summary["total_entries"] == 49
    assert summary["specific_entries_total"] == 6
    assert summary["inherited_wcag_entries_total"] == 43
    assert summary["inherited_wcag_registry_id"] == "wcag20aa_registry.v1"
    assert summary["specific_implemented_mapped_entry_count"] >= 1
    assert summary["implemented_mapped_result_counts"]["warn"] >= 1
    assert summary["mapped_entry_count"] + summary["unmapped_entry_count"] == 49
