from __future__ import annotations

import json
from pathlib import Path

import pytest

import fullbleed


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

    run_report = json.loads(Path(result.paths["run_report_path"]).read_text(encoding="utf-8"))
    assert run_report["pdf_ua_targeted"] is True
    assert run_report["engine_pdf_profile_requested"] == "pdfua"
    assert run_report["engine_pdf_profile_effective"] == "tagged"
    assert run_report["pdf_ua_seed_verify_path"]
    assert run_report["reading_order_trace_path"]
    assert run_report["pdf_structure_trace_path"]
    if "reading_order_trace_render_path" in run_report:
        assert run_report["reading_order_trace_render_path"]
        assert "reading_order_trace_cross_check" in run_report
    if "pdf_structure_trace_render_path" in run_report:
        assert run_report["pdf_structure_trace_render_path"]
        assert "pdf_structure_trace_cross_check" in run_report


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
