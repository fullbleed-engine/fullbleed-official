from __future__ import annotations

import json
from pathlib import Path

import pytest

import fullbleed

from fullbleed.audit_prototype import (
    prototype_verify_accessibility,
    prototype_verify_paged_media_rank,
    run_prototype_bundle,
)


ROOT = Path(__file__).resolve().parents[1]
SPECS = ROOT / "docs" / "specs"


def _write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def _render_preview_png(engine: "fullbleed.PdfEngine", html: str, css: str, out_dir: Path, stem: str = "preview") -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    if hasattr(engine, "render_image_pages_to_dir"):
        paths = list(engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
        if paths:
            return Path(paths[0])
    if hasattr(engine, "render_image_pages"):
        imgs = list(engine.render_image_pages(html, css, 144) or [])
        if imgs:
            path = out_dir / f"{stem}_page1.png"
            path.write_bytes(imgs[0])
            return path
    pytest.skip("engine PNG render preview API not available")


def _schema(path: str) -> dict:
    return json.loads((SPECS / path).read_text(encoding="utf-8"))


def _validate(jsonschema_module, schema: dict, payload: dict) -> None:
    jsonschema_module.Draft202012Validator(schema).validate(payload)


@pytest.fixture(scope="module")
def jsonschema_module():
    return pytest.importorskip("jsonschema")


def test_prototype_bundle_outputs_validate_against_schemas(tmp_path: Path, jsonschema_module) -> None:
    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en-US'><head><title>Test Doc</title>"
            "<link rel='stylesheet' href='doc.css'></head><body><main id='m1'>"
            "<div data-fb-a11y-signature-status='present'>Signature present: Jane Doe</div>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{font-family:Helvetica}")
    a11y_report = {
        "ok": True,
        "diagnostics": [],
    }
    component_validation = {
        "ok": True,
        "overflow_count": 0,
        "known_loss_count": 0,
    }
    parity_report = {
        "coverage": {
            "review_queue_items": 0,
        },
        "source_characteristics": {"page_count": 1},
    }
    run_report = {
        "metrics": {
            "source_page_count": 1,
            "render_page_count": 1,
        }
    }
    a11y_path = _write(tmp_path / "a11y.json", json.dumps(a11y_report))
    comp_path = _write(tmp_path / "comp.json", json.dumps(component_validation))
    parity_path = _write(tmp_path / "parity.json", json.dumps(parity_report))
    run_path = _write(tmp_path / "run.json", json.dumps(run_report))

    verifier, pmr = run_prototype_bundle(
        html_path=html,
        css_path=css,
        profile="cav",
        mode="error",
        a11y_report_path=a11y_path,
        component_validation_path=comp_path,
        parity_report_path=parity_path,
        run_report_path=run_path,
        expected_lang="en-US",
        expected_title="Test Doc",
    )

    _validate(jsonschema_module, _schema("fullbleed.a11y.verify.v1.schema.json"), verifier)
    _validate(jsonschema_module, _schema("fullbleed.pmr.v1.schema.json"), pmr)

    assert verifier["gate"]["ok"] is True
    assert verifier["coverage"]["wcag20aa"]["registry_id"] == "wcag20aa_registry.v1"
    assert verifier["coverage"]["wcag20aa"]["total_entries"] == 43
    assert verifier["coverage"]["section508"]["registry_id"] == "section508_html_registry.v1"
    assert verifier["coverage"]["section508"]["inherited_wcag_entries_total"] == 43
    assert verifier["observability"]["reported_finding_count"] == len(verifier["findings"])
    assert verifier["observability"]["stage_counts"]["post-emit"] >= 1
    assert verifier["observability"]["correlation_index"] == []
    assert verifier["wcag20aa_claim_readiness"]["target"] == "wcag20aa"
    assert verifier["wcag20aa_claim_readiness"]["claim_ready"] is False
    assert any(
        f["rule_id"] == "fb.a11y.claim.wcag20aa_level_readiness"
        for f in verifier["findings"]
    )
    assert any(
        row["pack_id"] == "wcag20aa.implemented_map.v1"
        for row in verifier["coverage"]["rule_pack_coverage"]
    )
    assert any(
        row["pack_id"] == "section508_html.implemented_map.v1"
        for row in verifier["coverage"]["rule_pack_coverage"]
    )
    assert pmr["gate"]["ok"] is True
    assert pmr["rank"]["score"] >= 90
    assert pmr["artifacts"]["css_linked"] is True
    assert pmr["observability"]["reported_audit_count"] == len(pmr["audits"])
    assert pmr["observability"]["correlation_index"] == []


def test_prototype_cav_regressions_fail_fast(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "bad.html",
        (
            "<!doctype html><html lang='en-US'><head><title>Bad Doc</title></head><body><main>"
            "<p>Review queue: 4 items pending</p>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "bad.css", "body{}")
    component_validation = {"overflow_count": 0, "known_loss_count": 0}
    parity_report = {"coverage": {"review_queue_items": 0}, "source_characteristics": {"page_count": 1}}
    run_report = {"metrics": {"source_page_count": 1, "render_page_count": 2}}

    verifier = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="cav",
        mode="error",
        parity_report=parity_report,
        expected_lang="en-US",
        expected_title="Bad Doc",
        generated_at="2026-02-24T00:00:00Z",
    )
    pmr = prototype_verify_paged_media_rank(
        html_path=html,
        css_path=css,
        profile="cav",
        mode="error",
        component_validation=component_validation,
        parity_report=parity_report,
        run_report=run_report,
        expected_lang="en-US",
        expected_title="Bad Doc",
        generated_at="2026-02-24T00:00:00Z",
    )

    assert verifier["gate"]["ok"] is False
    assert "fb.a11y.cav.document_only_content" in verifier["gate"]["failed_rule_ids"]
    assert pmr["gate"]["ok"] is False
    assert "pmr.layout.page_count_target" in pmr["gate"]["failed_audit_ids"]
    assert "pmr.cav.document_only_content" in pmr["gate"]["failed_audit_ids"]
    corr = {row["audit_id"]: row for row in pmr["observability"]["correlation_index"]}
    assert "pmr.layout.page_count_target" in corr
    assert "pmr.cav.document_only_content" in corr
    assert corr["pmr.layout.page_count_target"]["gate_failed"] is True
    assert corr["pmr.cav.document_only_content"]["gate_failed"] is True


def test_prototype_reports_are_deterministic_with_fixed_timestamp(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "doc.html",
        "<!doctype html><html lang='en'><head><title>X</title></head><body><main></main></body></html>",
    )
    css = _write(tmp_path / "doc.css", "body{}")
    kwargs = {
        "html_path": html,
        "css_path": css,
        "profile": "strict",
        "mode": "error",
        "generated_at": "2026-02-24T00:00:00Z",
    }
    v1 = prototype_verify_accessibility(**kwargs)
    v2 = prototype_verify_accessibility(**kwargs)
    p1 = prototype_verify_paged_media_rank(**kwargs)
    p2 = prototype_verify_paged_media_rank(**kwargs)
    assert v1 == v2
    assert p1 == p2
    assert p1["observability"]["reported_audit_count"] == len(p1["audits"])


def test_prototype_maps_heading_label_diagnostics_and_detects_post_emit_signals(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body>"
            "<main><h2></h2><label> </label><section role='region'></section></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{}")
    a11y_report = {
        "ok": False,
        "diagnostics": [
            {"code": "HEADING_EMPTY", "severity": "warning", "message": "Heading empty", "path": "/main/h2[1]"},
            {"code": "LABEL_EMPTY", "severity": "warning", "message": "Label empty", "path": "/main/label[1]"},
            {"code": "REGION_UNLABELED", "severity": "warning", "message": "Region unlabeled", "path": "/main/section[1]"},
        ],
    }
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        a11y_report=a11y_report,
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [
        f
        for f in report["findings"]
        if f["rule_id"] == "fb.a11y.headings_labels.present_nonempty"
    ]
    assert len(rows) == 1
    row = rows[0]
    assert row["verdict"] == "fail"  # canonical post-emit finding wins, severity preserved
    assert row["stage"] == "post-emit"
    assert len(row.get("related_ids") or []) >= 3  # pre-render bridged diagnostics correlated
    evidence_values = [e.get("values", {}) for e in row.get("evidence", [])]
    origin_stages = {
        str(v.get("correlated_origin_stage") or "")
        for v in evidence_values
        if "correlated_origin_stage" in v
    }
    assert {"post-emit", "pre-render"} <= origin_stages
    assert any(v.get("correlation_role") == "summary" for v in evidence_values)
    assert report["observability"]["dedup_event_count"] >= 1
    assert report["observability"]["correlated_finding_count"] >= 1
    assert any(
        item["rule_id"] == "fb.a11y.headings_labels.present_nonempty"
        and item["canonical_stage"] == "post-emit"
        and item["merged_pre_render_count"] >= 1
        for item in report["observability"]["correlation_index"]
    )


def test_prototype_detects_image_text_alternative_failures_and_title_only_warning(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body>"
            "<main><img src='a.png'><img src='b.png' title='Chart image'></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [f for f in report["findings"] if f["rule_id"] == "fb.a11y.images.alt_or_decorative"]
    assert rows
    assert any(f["verdict"] == "fail" for f in rows)
    assert any(
        f["evidence"][0]["values"]["image_missing_alt_count"] == 1
        and f["evidence"][0]["values"]["image_title_only_count"] == 1
        for f in rows
    )


def test_prototype_detects_unlabeled_form_controls(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body><main>"
            "<label for='good'>Full Name</label><input id='good'>"
            "<input id='bad'>"
            "<textarea aria-label='Notes'></textarea>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [
        f for f in report["findings"] if f["rule_id"] == "fb.a11y.forms.labels_or_instructions_present"
    ]
    assert rows
    assert any(f["verdict"] == "fail" for f in rows)
    assert any(
        f["evidence"][0]["values"]["form_control_count"] == 3
        and f["evidence"][0]["values"]["unlabeled_form_control_count"] == 1
        for f in rows
    )


def test_prototype_detects_invalid_controls_without_error_identification(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body><main>"
            "<input id='ok' aria-invalid='true' aria-describedby='err-ok'>"
            "<div id='err-ok'>Email is required</div>"
            "<input id='bad' aria-invalid='true'>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [
        f for f in report["findings"] if f["rule_id"] == "fb.a11y.forms.error_identification_present"
    ]
    assert rows
    assert any(f["verdict"] == "fail" for f in rows)
    assert any(
        f["evidence"][0]["values"]["invalid_form_control_count"] == 2
        and f["evidence"][0]["values"]["unidentified_error_form_control_count"] == 1
        for f in rows
    )


def test_prototype_detects_unnamed_and_generic_link_purpose_signals(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "links.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body><main>"
            "<a href='/empty'> </a>"
            "<a href='/generic'>Click here</a>"
            "<a href='/good'>Marriage record details</a>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "links.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [f for f in report["findings"] if f["rule_id"] == "fb.a11y.links.purpose_in_context"]
    assert rows
    assert any(f["verdict"] == "fail" for f in rows)
    assert any(
        f["evidence"][0]["values"]["link_count"] == 3
        and f["evidence"][0]["values"]["unnamed_link_count"] == 1
        and f["evidence"][0]["values"]["generic_link_text_count"] == 1
        for f in rows
    )


def test_prototype_warns_on_sensory_characteristics_instruction_phrases(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "sensory.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title></head><body><main>"
            "<p>See below for the details and sign on the right.</p>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "sensory.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [
        f
        for f in report["findings"]
        if f["rule_id"] == "fb.a11y.instructions.sensory_characteristics_seed"
    ]
    assert rows
    assert any(f["verdict"] == "warn" for f in rows)
    assert any(
        f["evidence"][0]["values"]["sensory_phrase_hit_count"] == 2
        and "see below" in f["evidence"][0]["values"]["sensory_phrase_hits"]
        and "on the right" in f["evidence"][0]["values"]["sensory_phrase_hits"]
        for f in rows
    )


def test_prototype_flags_invalid_language_of_parts_declarations(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "lang-parts.html",
        (
            "<!doctype html><html lang='en-US'><head><title>X</title></head><body>"
            "<main><p><span lang='es'>Hola</span> <span lang='-bad'>mundo</span></p></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "lang-parts.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [
        f
        for f in report["findings"]
        if f["rule_id"] == "fb.a11y.language.parts_declared_valid_seed"
    ]
    assert rows
    assert any(f["verdict"] == "fail" for f in rows)
    assert any(
        f["evidence"][0]["values"]["part_lang_attr_count"] == 2
        and f["evidence"][0]["values"]["invalid_part_lang_attr_count"] == 1
        for f in rows
    )


def test_prototype_warns_on_non_interference_risk_signals(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "active.html",
        (
            "<!doctype html><html lang='en'><head><title>X</title>"
            "<meta http-equiv='refresh' content='5'></head><body><main>"
            "<script>console.log('x')</script><div onclick='x()'>x</div>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "active.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [f for f in report["findings"] if f["rule_id"] == "fb.a11y.claim.non_interference_seed"]
    assert rows
    assert any(f["verdict"] == "warn" for f in rows)
    assert any(
        f["evidence"][0]["values"]["script_element_count"] == 1
        and f["evidence"][0]["values"]["inline_event_handler_attr_count"] == 1
        and f["evidence"][0]["values"]["meta_refresh_count"] == 1
        for f in rows
    )
    tech_rows = [
        f
        for f in report["findings"]
        if f["rule_id"] == "fb.a11y.claim.accessibility_supported_technologies_seed"
    ]
    assert tech_rows
    assert any(f["verdict"] == "warn" for f in tech_rows)
    assert any(
        f["evidence"][0]["values"]["script_element_count"] == 1
        and f["evidence"][0]["values"]["meta_refresh_count"] == 1
        for f in tech_rows
    )


def test_prototype_marks_complete_processes_scope_seed_manual_for_transactional_profile(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "txn.html",
        "<!doctype html><html lang='en-US'><head><title>X</title></head><body><main></main></body></html>",
    )
    css = _write(tmp_path / "txn.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="transactional",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )
    rows = [
        f
        for f in report["findings"]
        if f["rule_id"] == "fb.a11y.claim.complete_processes_scope_seed"
    ]
    assert rows
    assert any(f["verdict"] == "manual_needed" for f in rows)
    assert any(f["applicability"] == "applicable" for f in rows)
    assert any(
        f["evidence"][0]["values"]["profile"] == "transactional"
        and f["evidence"][0]["values"]["process_scope_declared"] is False
        for f in rows
    )


def test_prototype_emits_section508_e205_claim_seed_rules(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "s508.html",
        "<!doctype html><html lang='en-US'><head><title>X</title></head><body><main></main></body></html>",
    )
    css = _write(tmp_path / "s508.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )
    rows = {f["rule_id"]: f for f in report["findings"]}
    assert rows["fb.a11y.claim.section508.public_facing_content_applicability_seed"]["verdict"] == "manual_needed"
    assert rows["fb.a11y.claim.section508.official_communications_applicability_seed"]["verdict"] == "manual_needed"
    assert rows["fb.a11y.claim.section508.nara_exception_applicability_seed"]["verdict"] == "manual_needed"
    non_web = rows["fb.a11y.claim.section508.non_web_document_exceptions_html_seed"]
    assert non_web["verdict"] == "not_applicable"
    assert non_web["applicability"] == "not_applicable"
    assert any(
        ev["values"].get("delivery_target") == "html"
        for ev in non_web.get("evidence", [])
    )


def test_prototype_claim_evidence_can_satisfy_claim_seed_rules(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "claim.html",
        (
            "<!doctype html><html lang='en-US'><head><title>X</title></head><body>"
            "<main><a href='#x'>Go</a><input aria-label='Name'></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "claim.css", "body{}")
    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="cav",
        mode="error",
        claim_evidence={
            "technology_support": {"assessed": True, "basis_recorded": True},
            "wcag20": {
                "keyboard_assessed": True,
                "keyboard_basis_recorded": True,
                "keyboard_trap_assessed": True,
                "keyboard_trap_basis_recorded": True,
                "on_input_assessed": True,
                "on_input_basis_recorded": True,
                "on_focus_assessed": True,
                "on_focus_basis_recorded": True,
                "timing_adjustable_scope_declared": True,
                "timing_adjustable_assessed": True,
                "timing_adjustable_basis_recorded": True,
                "pause_stop_hide_scope_declared": True,
                "pause_stop_hide_assessed": True,
                "pause_stop_hide_basis_recorded": True,
                "three_flashes_scope_declared": True,
                "three_flashes_assessed": True,
                "three_flashes_basis_recorded": True,
                "audio_control_scope_declared": True,
                "audio_control_assessed": True,
                "audio_control_basis_recorded": True,
                "use_of_color_scope_declared": True,
                "use_of_color_assessed": True,
                "use_of_color_basis_recorded": True,
                "resize_text_scope_declared": True,
                "resize_text_assessed": True,
                "resize_text_basis_recorded": True,
                "images_of_text_scope_declared": True,
                "images_of_text_assessed": True,
                "images_of_text_basis_recorded": True,
                "prerecorded_av_alternative_scope_declared": True,
                "prerecorded_av_alternative_assessed": True,
                "prerecorded_av_alternative_basis_recorded": True,
                "prerecorded_captions_scope_declared": True,
                "prerecorded_captions_assessed": True,
                "prerecorded_captions_basis_recorded": True,
                "prerecorded_audio_description_or_media_alternative_scope_declared": True,
                "prerecorded_audio_description_or_media_alternative_assessed": True,
                "prerecorded_audio_description_or_media_alternative_basis_recorded": True,
                "live_captions_scope_declared": True,
                "live_captions_assessed": True,
                "live_captions_basis_recorded": True,
                "prerecorded_audio_description_scope_declared": True,
                "prerecorded_audio_description_assessed": True,
                "prerecorded_audio_description_basis_recorded": True,
                "meaningful_sequence_scope_declared": True,
                "meaningful_sequence_assessed": True,
                "meaningful_sequence_basis_recorded": True,
                "error_suggestion_scope_declared": True,
                "error_suggestion_assessed": True,
                "error_suggestion_basis_recorded": True,
                "error_prevention_scope_declared": True,
                "error_prevention_assessed": True,
                "error_prevention_basis_recorded": True,
                "consistent_identification_assessed": True,
                "consistent_identification_basis_recorded": True,
                "multiple_ways_scope_declared": True,
                "multiple_ways_assessed": True,
                "multiple_ways_basis_recorded": True,
                "consistent_navigation_scope_declared": True,
                "consistent_navigation_assessed": True,
                "consistent_navigation_basis_recorded": True,
            },
            "section508": {
                "scope_declared": True,
                "public_facing_determination_recorded": True,
                "official_communications_determination_recorded": True,
                "nara_exception_determination_recorded": True,
            },
        },
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = {f["rule_id"]: f for f in report["findings"]}
    assert rows["fb.a11y.claim.accessibility_supported_technologies_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.identification.consistent_identification_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.keyboard.operable_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.keyboard.no_trap_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.forms.on_input_behavior_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.focus.on_focus_behavior_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.timing.adjustable_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.timing.pause_stop_hide_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.seizures.three_flashes_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.audio.control_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.color.use_of_color_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.text.resize_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.images.of_text_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.media.prerecorded_audio_video_alternative_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.media.prerecorded_captions_seed"]["verdict"] == "pass"
    assert (
        rows["fb.a11y.media.prerecorded_audio_description_or_media_alternative_seed"][
            "verdict"
        ]
        == "pass"
    )
    assert rows["fb.a11y.media.live_captions_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.media.prerecorded_audio_description_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.sequence.meaningful_sequence_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.forms.error_suggestion_seed"]["verdict"] == "pass"
    assert (
        rows["fb.a11y.forms.error_prevention_legal_financial_data_seed"]["verdict"]
        == "pass"
    )
    assert rows["fb.a11y.navigation.multiple_ways_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.navigation.consistent_navigation_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.claim.section508.public_facing_content_applicability_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.claim.section508.official_communications_applicability_seed"]["verdict"] == "pass"
    assert rows["fb.a11y.claim.section508.nara_exception_applicability_seed"]["verdict"] == "pass"
    assert (
        rows["fb.a11y.claim.section508.non_web_document_exceptions_html_seed"]["verdict"]
        == "not_applicable"
    )
    tech_evidence = rows["fb.a11y.claim.accessibility_supported_technologies_seed"]["evidence"][0]["values"]
    assert tech_evidence["technology_support_assessed"] is True
    assert tech_evidence["technology_support_basis_recorded"] is True
    cid_evidence = rows["fb.a11y.identification.consistent_identification_seed"]["evidence"][0]["values"]
    assert cid_evidence["consistent_identification_assessed"] is True
    assert cid_evidence["consistent_identification_basis_recorded"] is True
    keyboard_evidence = rows["fb.a11y.keyboard.operable_seed"]["evidence"][0]["values"]
    assert keyboard_evidence["keyboard_assessed"] is True
    assert keyboard_evidence["keyboard_basis_recorded"] is True
    keyboard_trap_evidence = rows["fb.a11y.keyboard.no_trap_seed"]["evidence"][0]["values"]
    assert keyboard_trap_evidence["keyboard_trap_assessed"] is True
    assert keyboard_trap_evidence["keyboard_trap_basis_recorded"] is True
    on_input_evidence = rows["fb.a11y.forms.on_input_behavior_seed"]["evidence"][0]["values"]
    assert on_input_evidence["on_input_assessed"] is True
    assert on_input_evidence["on_input_basis_recorded"] is True
    on_focus_evidence = rows["fb.a11y.focus.on_focus_behavior_seed"]["evidence"][0]["values"]
    assert on_focus_evidence["on_focus_assessed"] is True
    assert on_focus_evidence["on_focus_basis_recorded"] is True
    timing_evidence = rows["fb.a11y.timing.adjustable_seed"]["evidence"][0]["values"]
    assert timing_evidence["timing_adjustable_scope_declared"] is True
    assert timing_evidence["timing_adjustable_assessed"] is True
    assert timing_evidence["timing_adjustable_basis_recorded"] is True
    pause_stop_hide_evidence = rows["fb.a11y.timing.pause_stop_hide_seed"]["evidence"][0]["values"]
    assert pause_stop_hide_evidence["pause_stop_hide_scope_declared"] is True
    assert pause_stop_hide_evidence["pause_stop_hide_assessed"] is True
    assert pause_stop_hide_evidence["pause_stop_hide_basis_recorded"] is True
    three_flashes_evidence = rows["fb.a11y.seizures.three_flashes_seed"]["evidence"][0]["values"]
    assert three_flashes_evidence["three_flashes_scope_declared"] is True
    assert three_flashes_evidence["three_flashes_assessed"] is True
    assert three_flashes_evidence["three_flashes_basis_recorded"] is True
    audio_control_evidence = rows["fb.a11y.audio.control_seed"]["evidence"][0]["values"]
    assert audio_control_evidence["audio_control_scope_declared"] is True
    assert audio_control_evidence["audio_control_assessed"] is True
    assert audio_control_evidence["audio_control_basis_recorded"] is True
    use_of_color_evidence = rows["fb.a11y.color.use_of_color_seed"]["evidence"][0]["values"]
    assert use_of_color_evidence["use_of_color_scope_declared"] is True
    assert use_of_color_evidence["use_of_color_assessed"] is True
    assert use_of_color_evidence["use_of_color_basis_recorded"] is True
    resize_text_evidence = rows["fb.a11y.text.resize_seed"]["evidence"][0]["values"]
    assert resize_text_evidence["resize_text_scope_declared"] is True
    assert resize_text_evidence["resize_text_assessed"] is True
    assert resize_text_evidence["resize_text_basis_recorded"] is True
    images_of_text_evidence = rows["fb.a11y.images.of_text_seed"]["evidence"][0]["values"]
    assert images_of_text_evidence["images_of_text_scope_declared"] is True
    assert images_of_text_evidence["images_of_text_assessed"] is True
    assert images_of_text_evidence["images_of_text_basis_recorded"] is True
    prerec_av_alt_evidence = rows[
        "fb.a11y.media.prerecorded_audio_video_alternative_seed"
    ]["evidence"][0]["values"]
    assert prerec_av_alt_evidence["prerecorded_av_alternative_scope_declared"] is True
    assert prerec_av_alt_evidence["prerecorded_av_alternative_assessed"] is True
    assert prerec_av_alt_evidence["prerecorded_av_alternative_basis_recorded"] is True
    prerec_captions_evidence = rows["fb.a11y.media.prerecorded_captions_seed"]["evidence"][0]["values"]
    assert prerec_captions_evidence["prerecorded_captions_scope_declared"] is True
    assert prerec_captions_evidence["prerecorded_captions_assessed"] is True
    assert prerec_captions_evidence["prerecorded_captions_basis_recorded"] is True
    prerec_ad_or_alt_evidence = rows[
        "fb.a11y.media.prerecorded_audio_description_or_media_alternative_seed"
    ]["evidence"][0]["values"]
    assert (
        prerec_ad_or_alt_evidence[
            "prerecorded_audio_description_or_media_alternative_scope_declared"
        ]
        is True
    )
    assert (
        prerec_ad_or_alt_evidence[
            "prerecorded_audio_description_or_media_alternative_assessed"
        ]
        is True
    )
    assert (
        prerec_ad_or_alt_evidence[
            "prerecorded_audio_description_or_media_alternative_basis_recorded"
        ]
        is True
    )
    live_captions_evidence = rows["fb.a11y.media.live_captions_seed"]["evidence"][0]["values"]
    assert live_captions_evidence["live_captions_scope_declared"] is True
    assert live_captions_evidence["live_captions_assessed"] is True
    assert live_captions_evidence["live_captions_basis_recorded"] is True
    prerec_ad_evidence = rows["fb.a11y.media.prerecorded_audio_description_seed"]["evidence"][0]["values"]
    assert prerec_ad_evidence["prerecorded_audio_description_scope_declared"] is True
    assert prerec_ad_evidence["prerecorded_audio_description_assessed"] is True
    assert prerec_ad_evidence["prerecorded_audio_description_basis_recorded"] is True
    meaningful_sequence_evidence = rows["fb.a11y.sequence.meaningful_sequence_seed"]["evidence"][0]["values"]
    assert meaningful_sequence_evidence["meaningful_sequence_scope_declared"] is True
    assert meaningful_sequence_evidence["meaningful_sequence_assessed"] is True
    assert meaningful_sequence_evidence["meaningful_sequence_basis_recorded"] is True
    error_suggestion_evidence = rows["fb.a11y.forms.error_suggestion_seed"]["evidence"][0]["values"]
    assert error_suggestion_evidence["error_suggestion_scope_declared"] is True
    assert error_suggestion_evidence["error_suggestion_assessed"] is True
    assert error_suggestion_evidence["error_suggestion_basis_recorded"] is True
    error_prevention_evidence = rows[
        "fb.a11y.forms.error_prevention_legal_financial_data_seed"
    ]["evidence"][0]["values"]
    assert error_prevention_evidence["error_prevention_scope_declared"] is True
    assert error_prevention_evidence["error_prevention_assessed"] is True
    assert error_prevention_evidence["error_prevention_basis_recorded"] is True
    multiple_ways_evidence = rows["fb.a11y.navigation.multiple_ways_seed"]["evidence"][0]["values"]
    assert multiple_ways_evidence["multiple_ways_scope_declared"] is True
    assert multiple_ways_evidence["multiple_ways_assessed"] is True
    assert multiple_ways_evidence["multiple_ways_basis_recorded"] is True
    consistent_nav_evidence = rows["fb.a11y.navigation.consistent_navigation_seed"]["evidence"][0]["values"]
    assert consistent_nav_evidence["consistent_navigation_scope_declared"] is True
    assert consistent_nav_evidence["consistent_navigation_assessed"] is True
    assert consistent_nav_evidence["consistent_navigation_basis_recorded"] is True


def test_prototype_focus_visible_seed_uses_css_focus_and_outline_signals(tmp_path: Path) -> None:
    html = _write(
        tmp_path / "focus.html",
        (
            "<!doctype html><html lang='en-US'><head><title>X</title></head><body>"
            "<main><a href='#x'>Link</a><input aria-label='Name'></main>"
            "</body></html>"
        ),
    )
    css_pass = _write(
        tmp_path / "focus-pass.css",
        "a:focus,input:focus{outline:2px solid #000}",
    )
    css_warn = _write(tmp_path / "focus-warn.css", "a,input{outline:none}")

    pass_report = prototype_verify_accessibility(
        html_path=html,
        css_path=css_pass,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )
    warn_report = prototype_verify_accessibility(
        html_path=html,
        css_path=css_warn,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )

    pass_row = next(
        f for f in pass_report["findings"] if f["rule_id"] == "fb.a11y.focus.visible_seed"
    )
    warn_row = next(
        f for f in warn_report["findings"] if f["rule_id"] == "fb.a11y.focus.visible_seed"
    )
    assert pass_row["verdict"] == "pass"
    assert warn_row["verdict"] == "warn"


def test_prototype_keyboard_seed_warns_on_pointer_only_custom_click_handlers(
    tmp_path: Path,
) -> None:
    html = _write(
        tmp_path / "keyboard-pointer-only.html",
        (
            "<!doctype html><html lang='en-US'><head><title>Keyboard</title></head><body>"
            "<main><div onclick='doThing()'>Open Panel</div></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "keyboard.css", "")

    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        claim_evidence={"wcag20": {"keyboard_assessed": True, "keyboard_basis_recorded": True}},
        generated_at="2026-02-24T00:00:00Z",
    )

    row = next(
        f for f in report["findings"] if f["rule_id"] == "fb.a11y.keyboard.operable_seed"
    )
    assert row["verdict"] == "warn"
    evidence = row["evidence"][0]["values"]
    assert evidence["interactive_keyboard_target_count"] == 0
    assert evidence["custom_click_handler_count"] == 1
    assert evidence["pointer_only_click_handler_count"] == 1


def test_prototype_focus_order_seed_warns_on_positive_tabindex_and_passes_without_it(
    tmp_path: Path,
) -> None:
    html_warn = _write(
        tmp_path / "focus-order-warn.html",
        (
            "<!doctype html><html lang='en-US'><head><title>X</title></head><body>"
            "<main><a href='#x' tabindex='2'>Link</a><input aria-label='Name'></main>"
            "</body></html>"
        ),
    )
    html_pass = _write(
        tmp_path / "focus-order-pass.html",
        (
            "<!doctype html><html lang='en-US'><head><title>X</title></head><body>"
            "<main><a href='#x'>Link</a><input aria-label='Name'></main>"
            "</body></html>"
        ),
    )
    css = _write(tmp_path / "focus-order.css", "body{}")
    warn_report = prototype_verify_accessibility(
        html_path=html_warn,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )
    pass_report = prototype_verify_accessibility(
        html_path=html_pass,
        css_path=css,
        profile="strict",
        mode="error",
        generated_at="2026-02-24T00:00:00Z",
    )
    warn_row = next(
        f for f in warn_report["findings"] if f["rule_id"] == "fb.a11y.focus.order_seed"
    )
    pass_row = next(
        f for f in pass_report["findings"] if f["rule_id"] == "fb.a11y.focus.order_seed"
    )
    assert warn_row["verdict"] == "warn"
    assert pass_row["verdict"] == "pass"
    assert warn_row["evidence"][0]["values"]["positive_tabindex_count"] == 1
    assert pass_row["evidence"][0]["values"]["positive_tabindex_count"] == 0
    assert pass_row["evidence"][0]["values"]["interactive_focus_target_count"] == 2


def test_prototype_warns_on_low_render_contrast_seed(tmp_path: Path) -> None:
    _require_pdf_engine()
    html_text = (
        "<!doctype html><html lang='en'><head><title>X</title></head><body>"
        "<main><p class='c'>Low Contrast Sample</p></main></body></html>"
    )
    css_text = (
        "body{margin:0;background:#fff}"
        "main{padding:24px}"
        ".c{margin:0;font:700 48px Helvetica, Arial, sans-serif;color:#999}"
    )
    html = _write(tmp_path / "contrast.html", html_text)
    css = _write(tmp_path / "contrast.css", css_text)
    engine = fullbleed.PdfEngine(document_lang="en", document_title="X")
    png_path = _render_preview_png(engine, html_text, css_text, tmp_path, stem="contrast")

    report = prototype_verify_accessibility(
        html_path=html,
        css_path=css,
        profile="strict",
        mode="error",
        render_preview_png_path=png_path,
        generated_at="2026-02-24T00:00:00Z",
    )

    rows = [f for f in report["findings"] if f["rule_id"] == "fb.a11y.contrast.minimum_render_seed"]
    assert rows
    assert any(f["verdict"] in {"warn", "manual_needed"} for f in rows)
    assert any(
        str(f["evidence"][0]["values"]["render_preview_png_path"]) == str(png_path)
        for f in rows
    )
