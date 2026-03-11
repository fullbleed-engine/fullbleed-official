from __future__ import annotations

import json
from pathlib import Path

import pytest

import fullbleed
from fullbleed.audit_prototype import prototype_verify_paged_media_rank


ROOT = Path(__file__).resolve().parents[1]
SPECS = ROOT / "docs" / "specs"


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def _write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


@pytest.fixture(scope="module")
def jsonschema_module():
    return pytest.importorskip("jsonschema")


def test_pdf_engine_verify_paged_media_rank_artifacts_emits_schema_valid_report(
    tmp_path: Path, jsonschema_module
) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en-US'><head><title>PMR Doc</title>"
            "<link rel='stylesheet' href='doc.css'></head><body><main id='root'>"
            "<table><caption>Rows</caption><tr><th scope='col'>Name</th><td>A</td></tr></table>"
            "<div data-fb-a11y-signature-status='present'>Signature present: Jane Doe</div>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{font-family:Helvetica}")

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="PMR Doc")
    report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=1,
        review_queue_items=0,
    )

    schema = json.loads((SPECS / "fullbleed.pmr.v1.schema.json").read_text(encoding="utf-8"))
    jsonschema_module.Draft202012Validator(schema).validate(report)

    assert report["schema"] == "fullbleed.pmr.v1"
    assert report["gate"]["ok"] is True
    assert report["rank"]["score"] >= 95
    assert report["artifacts"]["css_linked"] is True
    assert report["tooling"]["audit_contract_id"] == "fullbleed.audit_contract"
    assert report["tooling"]["audit_contract_version"] == "1"
    assert report["tooling"]["audit_contract_fingerprint"].startswith("sha256:")
    assert report["observability"]["reported_audit_count"] == len(report["audits"])
    assert report["observability"]["correlation_index"] == []
    assert any(a["audit_id"] == "pmr.layout.page_count_target" and a["verdict"] == "pass" for a in report["audits"])


def test_pdf_engine_verify_paged_media_rank_artifacts_prefers_pagination_trace_summary(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "doc.html",
        "<!doctype html><html lang='en-US'><head><title>PMR Pagination</title></head><body><main><p>Hello</p></main></body></html>",
    )
    css = _write(tmp_path / "doc.css", "body{font-family:Helvetica}")

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="PMR Pagination")
    report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=1,
        review_queue_items=0,
        pagination_trace_summary={
            "page_count": 2,
            "overflow_event_count": 3,
            "flowable_overlap_count": 1,
            "text_overlap_count": 2,
            "transition_count": 1,
        },
    )

    assert report["pagination_trace_summary"]["overflow_event_count"] == 3
    assert report["observability"]["signal_counts"]["pagination_overflow_event_count"] == 3
    assert report["observability"]["signal_counts"]["pagination_page_count"] == 2
    audits = {audit["audit_id"]: audit for audit in report["audits"]}
    assert audits["pmr.layout.overflow_none"]["verdict"] == "fail"
    assert audits["pmr.layout.overflow_none"]["evidence"][0]["diagnostic_ref"] == (
        "pagination_trace_summary.overflow_event_count"
    )
    assert audits["pmr.layout.page_count_target"]["verdict"] == "fail"
    assert audits["pmr.layout.page_count_target"]["evidence"][0]["diagnostic_ref"] == (
        "pagination_trace_summary.page_count"
    )


def test_pdf_engine_verify_paged_media_rank_artifacts_surfaces_diagnostic_reason_codes(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "doc.html",
        "<!doctype html><html lang='en-US'><head><title>PMR Diagnostics</title></head><body><main><p>Hello</p></main></body></html>",
    )
    css = _write(tmp_path / "doc.css", "body{font-family:Helvetica}")

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="PMR Diagnostics")
    report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=1,
        review_queue_items=0,
        pagination_trace_summary={
            "page_count": 2,
            "overflow_event_count": 1,
            "low_coverage_page_count": 1,
        },
        diagnostic_signals={
            "page_count_mismatch": True,
            "layout_collapse_detected": True,
            "pagination_overflow_detected": True,
            "token_fragmentation_detected": True,
            "typography_wrap_drift_detected": True,
            "semantic_table_alignment_drift": True,
            "low_coverage_page_count": 1,
            "token_fragmentation_block_count": 2,
            "wrap_drift_block_count": 3,
            "semantic_table_row_risk_count": 4,
            "fragmented_table_cell_count": 5,
        },
    )

    assert report["diagnostic_signals"]["page_count_mismatch"] is True
    assert report["diagnostic_signals"]["semantic_table_row_risk_count"] == 4
    assert set(report["gate"]["reason_codes"]) == {
        "page_count_mismatch",
        "layout_collapse_detected",
        "pagination_overflow_detected",
        "token_fragmentation_detected",
        "typography_wrap_drift_detected",
        "semantic_table_alignment_drift",
    }
    signals = report["observability"]["signal_counts"]
    assert signals["diagnostic_low_coverage_page_count"] == 1
    assert signals["diagnostic_token_fragmentation_block_count"] == 2
    assert signals["diagnostic_wrap_drift_block_count"] == 3
    assert signals["diagnostic_semantic_table_row_risk_count"] == 4
    assert signals["diagnostic_fragmented_table_cell_count"] == 5


def test_pdf_engine_verify_paged_media_rank_artifacts_promotes_metadata_mismatch_summary(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "doc.html",
        "<!doctype html><html lang='en'><head><title>DOM Title</title></head><body><main><p>Hello</p></main></body></html>",
    )
    css = _write(tmp_path / "doc.css", "body{font-family:Helvetica}")

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Engine Title")
    report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=1,
        review_queue_items=0,
    )

    summary = report["blocking_audit_summary"]
    assert any(
        row["audit_id"] == "pmr.doc.lang_present_valid"
        and row["failure_kind"] == "metadata_mismatch"
        and row["observed_value"] == "en"
        and row["expected_value"] == "en-US"
        and "metadata" in row["remediation_hint"].lower()
        for row in summary
    )
    assert any(
        row["audit_id"] == "pmr.doc.title_present_nonempty"
        and row["failure_kind"] == "metadata_mismatch"
        and row["observed_value"] == "DOM Title"
        and row["expected_value"] == "Engine Title"
        for row in summary
    )


def test_pdf_engine_verify_paged_media_rank_cav_fail_fast_regressions(tmp_path: Path) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "bad.html",
        (
            "<!doctype html><html lang='en-US'><head><title>Bad CAV</title></head><body><main>"
            "<p>Review queue: 4 items pending</p>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "bad.css", "body{}")
    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Bad CAV")
    report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=2,
        review_queue_items=0,
    )

    assert report["gate"]["ok"] is False
    failed = set(report["gate"]["failed_audit_ids"])
    assert "pmr.layout.page_count_target" in failed
    assert "pmr.cav.document_only_content" in failed
    corr = {row["audit_id"]: row for row in report["observability"]["correlation_index"]}
    assert "pmr.layout.page_count_target" in corr
    assert "pmr.cav.document_only_content" in corr
    assert corr["pmr.layout.page_count_target"]["gate_failed"] is True
    assert corr["pmr.cav.document_only_content"]["gate_failed"] is True


def test_engine_pmr_matches_prototype_for_seeded_audit_verdicts(tmp_path: Path) -> None:
    _require_pdf_engine()

    html = _write(
        tmp_path / "doc.html",
        (
            "<!doctype html><html lang='en-US'><head><title>Parity PMR</title></head><body><main>"
            "<table><tr><th>Name</th><td>A</td></tr></table>"
            "<div data-fb-a11y-signature-status='present'>Signature present: X</div>"
            "</main></body></html>"
        ),
    )
    css = _write(tmp_path / "doc.css", "body{}")
    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Parity PMR")

    engine_report = engine.verify_paged_media_rank_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        overflow_count=0,
        known_loss_count=0,
        source_page_count=1,
        render_page_count=1,
        review_queue_items=2,
        pagination_trace_summary={"page_count": 1, "overflow_event_count": 0},
    )
    proto_report = prototype_verify_paged_media_rank(
        html_path=html,
        css_path=css,
        profile="cav",
        mode="error",
        component_validation={"overflow_count": 0, "known_loss_count": 0},
        parity_report={"coverage": {"review_queue_items": 2}, "source_characteristics": {"page_count": 1}},
        run_report={"metrics": {"source_page_count": 1, "render_page_count": 1}},
        pagination_trace_summary={"page_count": 1, "overflow_event_count": 0},
        expected_lang="en-US",
        expected_title="Parity PMR",
        generated_at="2026-02-24T00:00:00Z",
    )

    def _verdicts(report: dict) -> dict[str, list[str]]:
        out: dict[str, list[str]] = {}
        for audit in report["audits"]:
            out.setdefault(audit["audit_id"], []).append(audit["verdict"])
        return out

    assert _verdicts(engine_report) == _verdicts(proto_report)
    assert engine_report["gate"]["ok"] == proto_report["gate"]["ok"]
    assert engine_report["gate"]["failed_audit_ids"] == proto_report["gate"]["failed_audit_ids"]
    assert engine_report["rank"]["score"] == pytest.approx(proto_report["rank"]["score"])
    assert engine_report["observability"]["verdict_counts"] == proto_report["observability"]["verdict_counts"]
    assert engine_report["observability"]["category_counts"] == proto_report["observability"]["category_counts"]
    assert {
        row["audit_id"] for row in engine_report["observability"]["correlation_index"]
    } == {
        row["audit_id"] for row in proto_report["observability"]["correlation_index"]
    }
