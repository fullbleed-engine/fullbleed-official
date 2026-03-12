from __future__ import annotations

import base64
import json
from pathlib import Path

import pytest

import fullbleed

REPO_ROOT = Path(__file__).resolve().parents[1]
TINY_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO7Z0ioAAAAASUVORK5CYII="
)


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def test_accessibility_engine_rejects_pdf_profile_override() -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    with pytest.raises(TypeError):
        AccessibilityEngine(pdf_profile="tagged")  # type: ignore[arg-type]


def test_accessibility_engine_strict_mode_requires_metadata(tmp_path: Path) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(strict=True, document_lang=None, document_title=None)
    with pytest.raises(ValueError):
        engine.render_bundle(
            body_html="<main><p>x</p></main>",
            css_text="body { color: #111; }",
            out_dir=str(tmp_path),
            stem="strict_meta",
            render_preview_png=False,
            run_verifier=False,
            run_pmr=False,
            run_pdf_ua_seed_verify=False,
            emit_reading_order_trace=False,
            emit_pdf_structure_trace=False,
        )


def test_accessibility_engine_metadata_guard_detects_emitted_dom_mismatch() -> None:
    from fullbleed.accessibility.engine import _validate_emitted_document_metadata

    with pytest.raises(ValueError, match="DOCUMENT_METADATA_MISMATCH"):
        _validate_emitted_document_metadata(
            "<!doctype html><html lang='en'><head><title>DOM Title</title></head><body></body></html>",
            {
                "document_lang": "en-US",
                "document_title": "Engine Title",
            },
        )


def test_accessibility_engine_strict_mode_fails_before_render_on_page_count_regression(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Pre-render Guard",
        document_css_href="pre_render_guard.css",
        strict=True,
    )
    out_dir = tmp_path / "pre_render_guard"
    body_html = "<main>" + "".join(f"<p>Row {idx}</p>" for idx in range(320)) + "</main>"

    with pytest.raises(
        ValueError,
        match="page count divergence detected|pre-render regression guard tripped",
    ):
        engine.render_bundle(
            body_html=body_html,
            css_text="@page { size: letter; margin: 0.5in; } p { margin: 0 0 14pt 0; font-size: 12pt; }",
            out_dir=str(out_dir),
            stem="pre_render_guard",
            source_analysis={"page_count": 1},
            render_preview_png=False,
            run_verifier=False,
            run_pmr=False,
            run_pdf_ua_seed_verify=False,
            emit_reading_order_trace=False,
            emit_pdf_structure_trace=False,
        )

    assert not (out_dir / "pre_render_guard.pdf").exists()


def test_accessibility_engine_css_metadata_emits_link_and_reports_fields(tmp_path: Path) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="CSS Metadata Runtime",
        document_css_href="styles/runtime.css",
        document_css_media="print",
        document_css_required=True,
        strict=False,
    )
    meta = engine.document_metadata()
    assert meta["document_css_href"] == "styles/runtime.css"
    assert meta["document_css_media"] == "print"
    assert meta["document_css_required"] is True

    out_dir = tmp_path / "css_meta_bundle"
    result = engine.render_bundle(
        body_html='<main data-fb-role="document-root"><h1>Title</h1><p>Hello</p></main>',
        css_text="@page { size: letter; }\nbody { color: #111; }",
        out_dir=str(out_dir),
        stem="css_meta",
        profile="strict",
        render_preview_png=False,
        run_verifier=False,
        run_pmr=False,
        run_pdf_ua_seed_verify=False,
        emit_reading_order_trace=False,
        emit_pdf_structure_trace=False,
    )

    html_text = Path(result.paths["html_path"]).read_text(encoding="utf-8")
    assert 'href="styles/runtime.css"' in html_text
    assert 'media="print"' in html_text

    run_report = json.loads(Path(result.paths["run_report_path"]).read_text(encoding="utf-8"))
    assert run_report["document_css_href"] == "styles/runtime.css"
    assert run_report["document_css_media"] == "print"
    assert run_report["document_css_required"] is True
    assert run_report["css_link_href"] == "styles/runtime.css"
    assert run_report["css_link_media"] == "print"


def test_accessibility_engine_verify_artifacts_promote_layout_guard_failures(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    html = tmp_path / "doc.html"
    html.write_text(
        "<!doctype html><html lang='en-US'><head><title>Guard Test</title></head><body><main id='root'><p>Hello</p></main></body></html>",
        encoding="utf-8",
    )
    css = tmp_path / "doc.css"
    css.write_text("body{font-family:Helvetica}", encoding="utf-8")

    page_row = {
        "page": 1,
        "occupied_area_ratio": 0.18,
        "dominant_owner": {
            "selector": "#sheet",
            "dom_path": "main > section:nth-of-type(1)",
            "component": "SheetSection",
            "role": "region",
            "tag": "section",
            "id": "sheet",
            "classes": "sheet-page",
        },
        "dominant_owner_label": "sheet-section",
        "dominant_owner_source": "flowable_bbox",
        "suspect_cause_codes": ["overflow_clipping", "fixed_height"],
    }
    pagination_trace = {
        "layout_collapse_diagnostics": {
            "detected": True,
            "page_count": 1,
            "pages": [page_row],
        },
        "pages": [dict(page_row, low_coverage=True)],
        "page_break_attribution": [],
        "dominant_break_triggers": [],
    }

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Guard Test",
        strict=False,
    )
    report = engine.verify_accessibility_artifacts(
        str(html),
        str(css),
        pagination_trace_summary={"page_count": 1, "low_coverage_page_count": 1},
        pagination_trace=pagination_trace,
        page_count_divergence={
            "source_page_count": 2,
            "render_page_count": 1,
            "delta": -1,
            "matches": False,
        },
        diagnostic_signals={
            "page_count_mismatch": True,
            "layout_collapse_detected": True,
        },
    )

    assert report["gate"]["ok"] is False
    assert "fb.a11y.layout.page_count_guard" in report["gate"]["failed_rule_ids"]
    assert "fb.a11y.layout.collapse_guard" in report["gate"]["failed_rule_ids"]
    collapse_row = next(
        row
        for row in report["blocking_issue_summary"]
        if row["rule_id"] == "fb.a11y.layout.collapse_guard" and row.get("page") == 1
    )
    assert collapse_row["selector"] == "#sheet"
    assert collapse_row["component"] == "SheetSection"
    assert collapse_row["owner_label"] == "sheet-section"
    assert "overflow_clipping" in collapse_row["suspect_cause_codes"]
    assert report["page_count_divergence"]["delta"] == -1


def test_accessibility_engine_verify_pmr_artifacts_promote_layout_guard_failures(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    html = tmp_path / "doc.html"
    html.write_text(
        "<!doctype html><html lang='en-US'><head><title>PMR Guard Test</title></head><body><main id='root'><p>Hello</p></main></body></html>",
        encoding="utf-8",
    )
    css = tmp_path / "doc.css"
    css.write_text("body{font-family:Helvetica}", encoding="utf-8")

    page_row = {
        "page": 2,
        "occupied_area_ratio": 0.14,
        "dominant_owner": {
            "selector": "#page-two",
            "dom_path": "main > section:nth-of-type(2)",
            "component": "PageTwoSection",
            "role": "region",
            "tag": "section",
            "id": "page-two",
            "classes": "page-section",
        },
        "dominant_owner_label": "page-two",
        "dominant_owner_source": "text_block",
        "suspect_cause_codes": ["page_root_height_mismatch"],
    }
    pagination_trace = {
        "layout_collapse_diagnostics": {
            "detected": True,
            "page_count": 1,
            "pages": [page_row],
        },
        "pages": [dict(page_row, low_coverage=True)],
        "page_break_attribution": [],
        "dominant_break_triggers": [],
    }

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="PMR Guard Test",
        strict=False,
    )
    report = engine.verify_pmr_artifacts(
        str(html),
        str(css),
        profile="cav",
        mode="error",
        component_validation={"overflow_count": 0, "known_loss_count": 0},
        source_analysis={"page_count": 1},
        render_page_count=1,
        pagination_trace_summary={"page_count": 1, "low_coverage_page_count": 1},
        pagination_trace=pagination_trace,
        page_count_divergence={
            "source_page_count": 1,
            "render_page_count": 3,
            "delta": 2,
            "matches": False,
        },
        diagnostic_signals={
            "page_count_mismatch": True,
            "layout_collapse_detected": True,
        },
    )

    assert report["gate"]["ok"] is False
    assert "pmr.layout.page_count_guard" in report["gate"]["failed_audit_ids"]
    assert "pmr.layout.collapse_guard" in report["gate"]["failed_audit_ids"]
    collapse_row = next(
        row
        for row in report["blocking_audit_summary"]
        if row["audit_id"] == "pmr.layout.collapse_guard" and row.get("page") == 2
    )
    assert collapse_row["selector"] == "#page-two"
    assert collapse_row["component"] == "PageTwoSection"
    assert collapse_row["owner_source"] == "text_block"


def test_accessibility_engine_verify_promotes_typography_spacing_guard(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    html = tmp_path / "doc.html"
    html.write_text(
        "<!doctype html><html lang='en-US'><head><title>Spacing Guard</title></head><body><main id='root'><p>Hello</p></main></body></html>",
        encoding="utf-8",
    )
    css = tmp_path / "doc.css"
    css.write_text("body{font-family:Helvetica}", encoding="utf-8")

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Spacing Guard",
        strict=False,
    )
    report = engine.verify_accessibility_artifacts(
        str(html),
        str(css),
        diagnostic_signals={
            "typography_spacing_drift_detected": True,
            "suspicious_char_width_block_count": 7,
        },
    )

    assert report["gate"]["ok"] is False
    assert "fb.a11y.typography.spacing_guard" in report["gate"]["failed_rule_ids"]
    row = next(
        row
        for row in report["blocking_issue_summary"]
        if row["rule_id"] == "fb.a11y.typography.spacing_guard"
    )
    assert row["failure_kind"] == "typography_spacing_drift"
    assert row["observed_value"] == "7"


def test_accessibility_engine_verify_promotes_sparse_page_header_guard(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    html = tmp_path / "doc.html"
    html.write_text(
        "<!doctype html><html lang='en-US'><head><title>Sparse Guard</title></head><body><main class='page'><p class='copy'>Case log entry 001</p></main></body></html>",
        encoding="utf-8",
    )
    css = tmp_path / "doc.css"
    css.write_text(
        "body{margin:0;background:#fff}.page{padding:18px 24px;font:18px Helvetica, Arial, sans-serif;color:#111}.copy{margin:0}",
        encoding="utf-8",
    )

    preview_engine = fullbleed.PdfEngine(
        document_lang="en-US",
        document_title="Sparse Guard Source",
    )
    source_html = (
        "<!doctype html><html lang='en-US'><head><title>Source</title></head><body>"
        "<main class='page'><div class='banner'></div><p class='copy'>Case log entry 001</p></main>"
        "</body></html>"
    )
    source_css = (
        "body{margin:0;background:#fff}"
        ".page{padding:18px 24px;font:18px Helvetica, Arial, sans-serif;color:#111}"
        ".banner{height:88px;background:#111;margin-bottom:22px}"
        ".copy{margin:0}"
    )
    if hasattr(preview_engine, "render_image_pages_to_dir"):
        source_paths = list(
            preview_engine.render_image_pages_to_dir(
                source_html, source_css, str(tmp_path), 144, "sparse_source"
            )
            or []
        )
        if not source_paths:
            pytest.skip("engine PNG render preview API not available")
        source_png = source_paths[0]
        render_paths = list(
            preview_engine.render_image_pages_to_dir(
                html.read_text(encoding="utf-8"),
                css.read_text(encoding="utf-8"),
                str(tmp_path),
                144,
                "sparse_render",
            )
            or []
        )
        if not render_paths:
            pytest.skip("engine PNG render preview API not available")
        render_png = render_paths[0]
    else:
        pytest.skip("engine PNG render preview API not available")

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Sparse Guard",
        strict=False,
    )
    report = engine.verify_accessibility_artifacts(
        str(html),
        str(css),
        source_preview_png_path=str(source_png),
        render_preview_png_path=str(render_png),
    )

    assert report["gate"]["ok"] is False
    assert "sparse_page_header_omission_detected" in report["gate"]["reason_codes"]
    assert "fb.a11y.visual.sparse_page_header_guard" in report["gate"]["failed_rule_ids"]
    assert report["sparse_page_visual_diagnostics"]["header_omission_detected"] is True
    row = next(
        row
        for row in report["blocking_issue_summary"]
        if row["rule_id"] == "fb.a11y.visual.sparse_page_header_guard"
    )
    assert row["failure_kind"] == "sparse_page_header_omission"
    assert int(row["observed_value"]) >= 4


def test_accessibility_engine_css_required_fails_without_href(tmp_path: Path) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Missing CSS Href",
        document_css_required=True,
        strict=False,
    )
    with pytest.raises(ValueError):
        engine.emit_html("<main><p>x</p></main>", str(tmp_path / "missing_href.html"))


def test_accessibility_engine_render_bundle_emits_pdfua_seed_and_trace_artifacts(tmp_path: Path) -> None:
    _require_pdf_engine()

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Accessibility Runtime Smoke",
        strict=False,
    )
    result = engine.render_bundle(
        body_html='<main data-fb-role="document-root"><h1>Title</h1><p>Hello</p></main>',
        css_text="@page { size: letter; }\nbody { color: #111; }",
        out_dir=str(tmp_path),
        stem="bundle_smoke",
        profile="strict",
        render_preview_png=False,
        run_verifier=True,
        run_pmr=True,
        run_pdf_ua_seed_verify=True,
        emit_reading_order_trace=True,
        emit_pdf_structure_trace=True,
    )

    assert result.pdf_ua_targeted is True
    assert result.paths["html_path"].endswith("bundle_smoke.html")
    assert result.paths["css_path"].endswith("bundle_smoke.css")
    assert result.paths["pdf_path"].endswith("bundle_smoke.pdf")
    assert "pdf_ua_seed_verify_path" in result.paths
    assert "reading_order_trace_path" in result.paths
    assert "pdf_structure_trace_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_reading_order_trace"):
        assert "reading_order_trace_render_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_structure_trace"):
        assert "pdf_structure_trace_render_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_font_resolution_trace"):
        assert "font_resolution_trace_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_pagination_trace"):
        assert "pagination_trace_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_typography_drift_trace"):
        assert "typography_drift_trace_path" in result.paths
    if hasattr(fullbleed.PdfEngine, "export_render_time_region_text_alignment_trace"):
        assert "region_text_alignment_trace_path" in result.paths

    html_text = Path(result.paths["html_path"]).read_text(encoding="utf-8")
    assert 'rel="stylesheet"' in html_text
    assert 'href="bundle_smoke.css"' in html_text
    assert "<html lang=\"en-US\">" in html_text
    assert "<title>Accessibility Runtime Smoke</title>" in html_text

    seed = json.loads(Path(result.paths["pdf_ua_seed_verify_path"]).read_text(encoding="utf-8"))
    assert seed["schema"] == "fullbleed.pdf.ua_seed_verify.v1"
    assert seed["seed_only"] is True
    assert "checks" in seed
    assert any(check["id"] == "pdf.structure_root.present" for check in seed["checks"])
    if "reading_order_trace_render_path" in result.paths:
        assert any(
            check["id"] == "pdf.trace.reading_order.render_time.emitted"
            for check in seed["checks"]
        )
        assert any(
            check["id"] == "pdf.trace.reading_order.cross_check_seed"
            for check in seed["checks"]
        )
    if "pdf_structure_trace_render_path" in result.paths:
        assert any(
            check["id"] == "pdf.trace.structure.render_time.emitted"
            for check in seed["checks"]
        )
        assert any(
            check["id"] == "pdf.trace.structure.render_time.tag_balance_seed"
            for check in seed["checks"]
        )
        assert any(
            check["id"] == "pdf.trace.structure.render_time.tagged_text_presence_seed"
            for check in seed["checks"]
        )
        assert any(
            check["id"] == "pdf.trace.structure.render_time.untagged_text_ratio_seed"
            for check in seed["checks"]
        )
        assert any(
            check["id"] == "pdf.trace.structure.cross_check_seed"
            for check in seed["checks"]
        )

    reading = json.loads(Path(result.paths["reading_order_trace_path"]).read_text(encoding="utf-8"))
    assert reading["schema"] == "fullbleed.pdf.reading_order_trace.v1"
    assert reading["schema_version"] == 1
    assert "summary" in reading
    if hasattr(fullbleed, "export_pdf_reading_order_trace"):
        assert reading["extractor"] == "lopdf"

    structure = json.loads(Path(result.paths["pdf_structure_trace_path"]).read_text(encoding="utf-8"))
    assert structure["schema"] == "fullbleed.pdf.structure_trace.v1"
    assert structure["schema_version"] == 1
    assert "token_counts" in structure
    if hasattr(fullbleed, "export_pdf_structure_trace"):
        assert structure["extractor"] == "lopdf"

    if "reading_order_trace_render_path" in result.paths:
        reading_render = json.loads(
            Path(result.paths["reading_order_trace_render_path"]).read_text(encoding="utf-8")
        )
        assert reading_render["schema"] == "fullbleed.pdf.reading_order_trace.v1"
        assert reading_render["extractor"] == "render_time_commands"
        assert "summary" in reading_render

    if "pdf_structure_trace_render_path" in result.paths:
        structure_render = json.loads(
            Path(result.paths["pdf_structure_trace_render_path"]).read_text(encoding="utf-8")
        )
        assert structure_render["schema"] == "fullbleed.pdf.structure_trace.v1"
        assert structure_render["extractor"] == "render_time_commands"
        assert "summary" in structure_render
    if "font_resolution_trace_path" in result.paths:
        font_trace = json.loads(
            Path(result.paths["font_resolution_trace_path"]).read_text(encoding="utf-8")
        )
        assert font_trace["schema"] == "fullbleed.font_resolution_trace.v1"
        assert font_trace["schema_version"] == 1
        assert font_trace["extractor"] == "render_time_commands"
        assert font_trace["summary"]["font_count"] >= 1
    if "pagination_trace_path" in result.paths:
        pagination_trace = json.loads(
            Path(result.paths["pagination_trace_path"]).read_text(encoding="utf-8")
        )
        assert pagination_trace["schema"] == "fullbleed.pagination_trace.v1"
        assert pagination_trace["schema_version"] == 1
        assert pagination_trace["summary"]["page_count"] >= 1
    if "typography_drift_trace_path" in result.paths:
        typography_trace = json.loads(
            Path(result.paths["typography_drift_trace_path"]).read_text(encoding="utf-8")
        )
        assert typography_trace["schema"] == "fullbleed.typography_drift_trace.v1"
        assert typography_trace["schema_version"] == 1
        assert typography_trace["summary"]["block_count"] >= 1
    if "region_text_alignment_trace_path" in result.paths:
        region_trace = json.loads(
            Path(result.paths["region_text_alignment_trace_path"]).read_text(encoding="utf-8")
        )
        assert region_trace["schema"] == "fullbleed.region_text_alignment_trace.v1"
        assert region_trace["schema_version"] == 1
        assert "summary" in region_trace

    run_report = json.loads(Path(result.paths["run_report_path"]).read_text(encoding="utf-8"))
    assert run_report["pdf_ua_targeted"] is True
    assert run_report["engine_pdf_profile_requested"] == "pdfua"
    assert run_report["engine_pdf_profile_effective"] == "tagged"
    assert run_report["pdf_ua_seed_verify_path"]
    assert run_report["reading_order_trace_path"]
    assert run_report["pdf_structure_trace_path"]
    assert run_report["deliverables"]["html_path"] == "bundle_smoke.html"
    assert run_report["deliverables"]["css_path"] == "bundle_smoke.css"
    assert run_report["deliverables"]["pdf_path"] == "bundle_smoke.pdf"
    assert run_report["deliverables"]["run_report_path"] == "bundle_smoke_run_report.json"
    if "font_resolution_trace_path" in result.paths:
        assert run_report["font_resolution_trace_path"]
        assert run_report["deliverables"]["font_resolution_trace_path"] == (
            "bundle_smoke_font_resolution_trace.json"
        )
        assert run_report["font_resolution_summary"]["font_count"] >= 1
    if "pagination_trace_path" in result.paths:
        assert run_report["pagination_trace_path"]
        assert run_report["deliverables"]["pagination_trace_path"] == (
            "bundle_smoke_pagination_trace.json"
        )
        assert run_report["pagination_trace_summary"]["page_count"] >= 1
        verifier_report = json.loads(
            Path(result.paths["engine_a11y_verify_path"]).read_text(encoding="utf-8")
        )
        pmr_report = json.loads(
            Path(result.paths["engine_pmr_path"]).read_text(encoding="utf-8")
        )
        assert verifier_report["pagination_trace_summary"]["page_count"] >= 1
        assert pmr_report["pagination_trace_summary"]["page_count"] >= 1
        assert (
            verifier_report["observability"]["signal_counts"]["pagination_page_count"] >= 1
        )
        assert pmr_report["observability"]["signal_counts"]["pagination_page_count"] >= 1
        assert verifier_report["diagnostic_signals"]["page_count_mismatch"] is False
        assert pmr_report["diagnostic_signals"]["page_count_mismatch"] is False
        assert isinstance(verifier_report["gate"]["reason_codes"], list)
        assert isinstance(pmr_report["gate"]["reason_codes"], list)
        assert verifier_report["layout_diagnostics"]["page_ownership"]
        assert pmr_report["layout_diagnostics"]["page_ownership"]
        assert run_report["layout_collapse_summary"]["page_ownership"]
        assert (
            verifier_report["layout_diagnostics"]["page_ownership"][0][
                "dominant_owner_label"
            ]
            .startswith("main")
        )
        assert run_report["a11y_issue_summary"] == verifier_report["blocking_issue_summary"]
    if "typography_drift_trace_path" in result.paths:
        assert run_report["typography_drift_trace_path"]
        assert run_report["deliverables"]["typography_drift_trace_path"] == (
            "bundle_smoke_typography_drift_trace.json"
        )
        assert run_report["typography_drift_summary"]["block_count"] >= 1
    if "region_text_alignment_trace_path" in result.paths:
        assert run_report["region_text_alignment_trace_path"]
        assert run_report["deliverables"]["region_text_alignment_trace_path"] == (
            "bundle_smoke_region_text_alignment_trace.json"
        )
        assert "region_text_alignment_summary" in run_report
    assert run_report["diagnostic_signals"]["page_count_mismatch"] is False
    if "reading_order_trace_render_path" in run_report:
        assert run_report["reading_order_trace_render_path"]
        assert "reading_order_trace_cross_check" in run_report
        assert run_report["deliverables"]["reading_order_trace_render_path"]
    if "pdf_structure_trace_render_path" in run_report:
        assert run_report["pdf_structure_trace_render_path"]
        assert "pdf_structure_trace_cross_check" in run_report
        assert run_report["deliverables"]["pdf_structure_trace_render_path"]


def test_pdf_engine_font_resolution_trace_reports_registered_file_targets() -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_font_resolution_trace"):
        pytest.skip("font resolution trace export is not available in this build")

    font_path = REPO_ROOT / "python" / "fullbleed_assets" / "fonts" / "Inter-Variable.ttf"
    engine = fullbleed.PdfEngine(font_files=[str(font_path)])
    trace = engine.export_render_time_font_resolution_trace(
        "<main><p style=\"font-family: 'Inter'\">Hello trace</p></main>",
        "@page { size: letter; } body { color: #111; }",
    )

    assert trace["schema"] == "fullbleed.font_resolution_trace.v1"
    assert trace["schema_version"] == 1
    assert trace["summary"]["font_count"] >= 1
    inter_entry = next(font for font in trace["fonts"] if font["requested_name"] == "Inter")
    assert inter_entry["deterministic"] is True
    assert inter_entry["pdf_target"]["source"] == "file"
    assert inter_entry["pdf_target"]["resolved_file_name"] == "Inter-Variable.ttf"
    assert inter_entry["raster_target"]["source"] == "file"
    assert inter_entry["raster_target"]["resolved_file_name"] == "Inter-Variable.ttf"


def test_pdf_engine_font_resolution_trace_reports_missing_font_fallbacks() -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_font_resolution_trace"):
        pytest.skip("font resolution trace export is not available in this build")

    engine = fullbleed.PdfEngine()
    trace = engine.export_render_time_font_resolution_trace(
        "<main><p style=\"font-family: 'DefinitelyMissingFont'\">Hello trace</p></main>",
        "@page { size: letter; } body { color: #111; }",
    )

    missing_entry = next(
        font for font in trace["fonts"] if font["requested_name"] == "DefinitelyMissingFont"
    )
    assert missing_entry["deterministic"] is False
    assert missing_entry["fallback_reason"] == "unregistered_primary_fallback"
    assert missing_entry["pdf_target"]["outcome"] == "base14_fallback"
    assert missing_entry["raster_target"]["outcome"] == "system_fallback"
    assert trace["summary"]["raster_system_fallback_count"] >= 1


def test_pdf_engine_pagination_trace_reports_page_transitions() -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_pagination_trace"):
        pytest.skip("pagination trace export is not available in this build")

    rows = "".join(f"<p>Row {idx}</p>" for idx in range(220))
    engine = fullbleed.PdfEngine()
    trace = engine.export_render_time_pagination_trace(
        f"<main>{rows}</main>",
        "@page { size: letter; margin: 0.5in; } p { margin: 0 0 14pt 0; font-size: 12pt; }",
    )

    assert trace["schema"] == "fullbleed.pagination_trace.v1"
    assert trace["schema_version"] == 1
    assert trace["summary"]["page_count"] >= 2
    assert trace["summary"]["transition_count"] >= 1
    assert trace["summary"]["placement_count"] >= 1
    assert any(event["event_type"] == "transition" for event in trace["events"])


def test_pdf_engine_asset_resolution_trace_resolves_file_uri_sources(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_asset_resolution_trace"):
        pytest.skip("asset resolution trace export is not available in this build")

    image_path = tmp_path / "tiny.png"
    image_path.write_bytes(TINY_PNG)

    engine = fullbleed.PdfEngine()
    trace = engine.export_render_time_asset_resolution_trace(
        f'<main><img src="{image_path.as_uri()}" alt="Tiny pixel"></main>',
        "@page { size: letter; } body { color: #111; }",
    )

    assert trace["schema"] == "fullbleed.asset_resolution_trace.v1"
    assert trace["summary"]["resolved_count"] == 1
    entry = trace["assets"][0]
    assert entry["resolver"] == "file_uri"
    assert entry["success"] is True
    assert entry["render_outcome"] == "raster_image"


def test_accessibility_engine_render_bundle_emits_asset_resolution_trace_for_bundle_images(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_asset_resolution_trace"):
        pytest.skip("asset resolution trace export is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    image_path = tmp_path / "bundle_tiny.png"
    image_path.write_bytes(TINY_PNG)
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(image_path), "image")

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Bundle Image Trace",
        strict=False,
    )
    engine.raw_engine.register_bundle(bundle)
    result = engine.render_bundle(
        body_html='<main data-fb-role="document-root"><img src="bundle_tiny.png" alt="Tiny pixel"></main>',
        css_text="@page { size: letter; }\nimg { width: 24px; height: 24px; }",
        out_dir=str(tmp_path / "bundle_trace"),
        stem="bundle_trace",
        render_preview_png=False,
        run_verifier=False,
        run_pmr=False,
        run_pdf_ua_seed_verify=False,
        emit_reading_order_trace=False,
        emit_pdf_structure_trace=False,
    )

    assert "asset_resolution_trace_path" in result.paths
    trace = json.loads(Path(result.paths["asset_resolution_trace_path"]).read_text(encoding="utf-8"))
    entry = trace["assets"][0]
    assert entry["resolver"] == "bundle"
    assert entry["success"] is True
    assert entry["asset_name"] == "bundle_tiny.png"

    run_report = json.loads(Path(result.paths["run_report_path"]).read_text(encoding="utf-8"))
    assert run_report["deliverables"]["asset_resolution_trace_path"] == (
        "bundle_trace_asset_resolution_trace.json"
    )
    assert run_report["asset_resolution_summary"]["bundle_resolved_count"] == 1


def test_accessibility_engine_strict_mode_fails_on_unresolved_image_sources(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_asset_resolution_trace"):
        pytest.skip("asset resolution trace export is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Strict Missing Image",
        document_css_required=False,
        strict=True,
    )
    with pytest.raises(ValueError, match="unresolved image source"):
        engine.render_bundle(
            body_html='<main data-fb-role="document-root"><img src="missing-image.png" alt="Missing"></main>',
            css_text="@page { size: letter; }",
            out_dir=str(tmp_path / "strict_missing_image"),
            stem="strict_missing_image",
            render_preview_png=False,
            run_verifier=False,
            run_pmr=False,
            run_pdf_ua_seed_verify=False,
            emit_reading_order_trace=False,
            emit_pdf_structure_trace=False,
        )


def test_accessibility_engine_strict_mode_fails_on_page_count_divergence(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_pagination_trace"):
        pytest.skip("pagination trace export is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Strict Page Divergence",
        document_css_required=False,
        strict=True,
    )
    with pytest.raises(ValueError, match="page count divergence detected"):
        engine.render_bundle(
            body_html='<main data-fb-role="document-root"><p>Single page</p></main>',
            css_text="@page { size: letter; }",
            out_dir=str(tmp_path / "strict_page_divergence"),
            stem="strict_page_divergence",
            source_analysis={"page_count": 2},
            render_preview_png=False,
            run_verifier=False,
            run_pmr=False,
            run_pdf_ua_seed_verify=False,
            emit_reading_order_trace=False,
            emit_pdf_structure_trace=False,
        )


def test_accessibility_engine_render_bundle_promotes_page_break_owner_diagnostics(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed.PdfEngine, "export_render_time_pagination_trace"):
        pytest.skip("pagination trace export is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Ownership Diagnostics",
        strict=False,
    )
    result = engine.render_bundle(
        body_html=(
            '<main data-fb-role="document-root">'
            '<section data-fb-component="intro"><h1>Intro</h1><p>Alpha</p></section>'
            '<section data-fb-component="appendix" style="page-break-before: always;">'
            "<h2>Appendix</h2><p>Bravo</p></section>"
            "</main>"
        ),
        css_text="@page { size: letter; }\nbody { color: #111; }",
        out_dir=str(tmp_path / "ownership_bundle"),
        stem="ownership_bundle",
        render_preview_png=False,
        run_verifier=True,
        run_pmr=True,
        run_pdf_ua_seed_verify=False,
        emit_reading_order_trace=False,
        emit_pdf_structure_trace=False,
    )

    verifier_report = json.loads(
        Path(result.paths["engine_a11y_verify_path"]).read_text(encoding="utf-8")
    )
    pmr_report = json.loads(Path(result.paths["engine_pmr_path"]).read_text(encoding="utf-8"))

    assert any(
        row["dominant_owner_label"] == "appendix"
        for row in verifier_report["layout_diagnostics"]["page_ownership"]
    )
    assert any(
        row["dominant_owner_label"] == "appendix"
        for row in pmr_report["layout_diagnostics"]["page_ownership"]
    )


def test_accessibility_engine_definition_list_text_is_tagged_in_render_trace(
    tmp_path: Path,
) -> None:
    _require_pdf_engine()

    if not hasattr(fullbleed.PdfEngine, "export_render_time_structure_trace"):
        pytest.skip("render-time structure trace export is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="Definition List Tagging",
        strict=False,
    )
    result = engine.render_bundle(
        body_html=(
            '<main data-fb-role="document-root">'
            '<h1>Record Header</h1>'
            '<dl>'
            '<dt>Agency</dt><dd>Department of Health</dd>'
            '<dt>Jurisdiction</dt><dd>State of Florida</dd>'
            '</dl>'
            '</main>'
        ),
        css_text="@page { size: letter; }\nbody { color: #111; } dl { margin: 0; }",
        out_dir=str(tmp_path),
        stem="dl_tagging",
        profile="strict",
        render_preview_png=False,
        run_verifier=False,
        run_pmr=False,
        run_pdf_ua_seed_verify=False,
        emit_reading_order_trace=False,
        emit_pdf_structure_trace=True,
    )

    structure_render = json.loads(
        Path(result.paths["pdf_structure_trace_render_path"]).read_text(encoding="utf-8")
    )
    assert structure_render["extractor"] == "render_time_commands"
    assert structure_render["summary"]["tagged_text_draw_count"] >= 5
    assert structure_render["summary"]["untagged_text_draw_count"] == 0
    token_counts = dict(structure_render.get("token_counts") or {})
    assert token_counts.get("L", 0) >= 1
    assert token_counts.get("LI", 0) >= 2
    assert token_counts.get("Lbl", 0) >= 2
    assert token_counts.get("LBody", 0) >= 2


def test_native_pdf_page_text_extraction_uses_engine_extension(tmp_path: Path) -> None:
    _require_pdf_engine()
    if not hasattr(fullbleed, "extract_pdf_page_texts"):
        pytest.skip("native pdf page text extraction is not available in this build")

    from fullbleed.accessibility import AccessibilityEngine

    engine = AccessibilityEngine(
        document_lang="en-US",
        document_title="PDF Text Extract Smoke",
        strict=False,
    )
    result = engine.render_bundle(
        body_html='<main data-fb-role="document-root"><h1>Packet Title</h1><p>Alpha Beta</p></main>',
        css_text="@page { size: letter; } body { color: #111; }",
        out_dir=str(tmp_path),
        stem="pdf_text_extract",
        profile="strict",
        render_preview_png=False,
        run_verifier=False,
        run_pmr=False,
        run_pdf_ua_seed_verify=False,
        emit_reading_order_trace=False,
        emit_pdf_structure_trace=False,
    )
    report = fullbleed.extract_pdf_page_texts(result.paths["pdf_path"])
    assert report["schema"] == "fullbleed.pdf.page_text_extract.v1"
    assert report["extractor"] == "lopdf"
    assert report["ok"] is True
    assert report["summary"]["page_count"] >= 1
    assert len(report["pages"]) >= 1
    assert "Packet Title" in (report["pages"][0]["text"] or "")
