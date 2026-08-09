from __future__ import annotations

import inspect
import urllib.request

import pytest

import fullbleed
import fullbleed._fullbleed as native


PUBLIC_NATIVE_NAMES = {
    "PdfEngine",
    "CompiledDocument",
    "AssetKind",
    "Asset",
    "AssetBundle",
    "WatermarkSpec",
    "inspect_pdf",
    "build_features",
    "inspect_template_catalog",
    "vendored_asset",
    "fetch_asset",
    "concat_css",
    "audit_contract_metadata",
    "audit_contract_registry",
    "audit_contract_wcag20aa_coverage",
    "audit_contract_section508_html_coverage",
    "audit_contrast_render_png",
    "audit_sparse_page_visual_pair",
    "extract_pdf_page_texts",
    "export_pdf_reading_order_trace",
    "export_pdf_structure_trace",
    "verify_pdf_ua_seed",
    "finalize_stamp_pdf",
    "finalize_compose_pdf",
}


def test_stable_abi_facade_preserves_native_import_surface() -> None:
    assert callable(native._dispatch)
    assert set(native.__all__) == PUBLIC_NATIVE_NAMES
    assert PUBLIC_NATIVE_NAMES < set(fullbleed.__all__)
    assert fullbleed.PdfEngine is native.PdfEngine
    assert fullbleed.AssetBundle is native.AssetBundle
    classes = {
        "PdfEngine",
        "CompiledDocument",
        "AssetKind",
        "Asset",
        "AssetBundle",
        "WatermarkSpec",
    }
    assert all(getattr(fullbleed, name).__module__ == "builtins" for name in classes)
    assert all(
        getattr(fullbleed, name).__module__ == "fullbleed._fullbleed"
        for name in PUBLIC_NATIVE_NAMES - classes
    )


def test_pdf_engine_constructor_and_method_signatures_remain_explicit() -> None:
    constructor = inspect.signature(fullbleed.PdfEngine)
    assert list(constructor.parameters) == [
        "page_width",
        "page_height",
        "margin",
        "page_margins",
        "font_dirs",
        "font_files",
        "reuse_xobjects",
        "svg_form_xobjects",
        "svg_raster_fallback",
        "unicode_support",
        "shape_text",
        "unicode_metrics",
        "pdf_version",
        "pdf_profile",
        "output_intent_icc",
        "output_intent_identifier",
        "output_intent_info",
        "output_intent_components",
        "color_space",
        "document_lang",
        "document_title",
        "header_first",
        "header_each",
        "header_last",
        "header_x",
        "header_y_from_top",
        "header_font_name",
        "header_font_size",
        "header_color",
        "header_html_first",
        "header_html_each",
        "header_html_last",
        "header_html_x",
        "header_html_y_from_top",
        "header_html_width",
        "header_html_height",
        "footer_first",
        "footer_each",
        "footer_last",
        "footer_x",
        "footer_y_from_bottom",
        "footer_font_name",
        "footer_font_size",
        "footer_color",
        "watermark",
        "watermark_text",
        "watermark_html",
        "watermark_image",
        "watermark_layer",
        "watermark_semantics",
        "watermark_opacity",
        "watermark_rotation",
        "watermark_font_name",
        "watermark_font_size",
        "watermark_color",
        "paginated_context",
        "template_binding",
        "layout_strategy",
        "accept_lazy_layout_cost",
        "lazy_max_passes",
        "lazy_budget_ms",
        "jit_mode",
        "debug",
        "debug_out",
        "perf",
        "perf_out",
    ]
    assert constructor.parameters["reuse_xobjects"].default is True
    assert constructor.parameters["watermark_layer"].default == "overlay"
    assert constructor.parameters["lazy_max_passes"].default == 4
    assert constructor.parameters["lazy_budget_ms"].default == 50.0
    assert str(inspect.signature(fullbleed.PdfEngine.render_pdf)) == (
        "(self, /, html, css, deterministic_hash=None)"
    )
    assert str(inspect.signature(fullbleed.PdfEngine.compile_pdf)) == (
        "(self, /, html, css)"
    )
    assert str(inspect.signature(fullbleed.CompiledDocument.render_pdf)) == (
        "(self, /, deterministic_hash=None)"
    )
    assert str(inspect.signature(fullbleed.CompiledDocument.render_pdf_bindings)) == (
        "(self, /, bindings, deterministic_hash=None)"
    )
    assert str(inspect.signature(fullbleed.PdfEngine.verify_accessibility_html)) == (
        "(self, /, html, css='', profile='strict', mode='error', "
        "render_preview_png_path=None, a11y_report=None, claim_evidence=None, "
        "pagination_trace_summary=None, diagnostic_signals=None)"
    )


def test_value_wrappers_preserve_construction_and_validation_contracts() -> None:
    assert fullbleed.AssetKind.Css == "css"
    assert fullbleed.AssetKind.Font == "font"
    assert fullbleed.AssetKind.Image == "image"
    assert fullbleed.AssetKind.Pdf == "pdf"
    assert fullbleed.AssetKind.Svg == "svg"
    assert fullbleed.AssetKind.Other == "other"
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.AssetKind()
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.Asset()
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.CompiledDocument()
    with pytest.raises(TypeError, match="not an acceptable base type"):
        class AssetSubclass(fullbleed.Asset):
            pass
    with pytest.raises(TypeError, match="kind must be a string"):
        fullbleed.WatermarkSpec(1, "DRAFT")
    with pytest.raises(TypeError, match="opacity must be a real number"):
        fullbleed.WatermarkSpec("text", "DRAFT", opacity="0.5")


def test_facade_routes_assets_and_pdf_rendering_through_capsules(tmp_path) -> None:
    css_path = tmp_path / "bundle.css"
    css_path.write_text("p { color: rebeccapurple; }", encoding="utf-8")

    asset = fullbleed.vendored_asset(str(css_path), fullbleed.AssetKind.Css)
    assert type(asset._handle).__name__ == "PyCapsule"
    assert asset.info() == {
        "name": "bundle.css",
        "kind": "css",
        "bytes": css_path.stat().st_size,
        "trusted": False,
        "source": str(css_path),
    }

    bundle = fullbleed.AssetBundle()
    bundle.add(asset)
    assert "rebeccapurple" in bundle.css()

    engine = fullbleed.PdfEngine(
        document_lang="en-US",
        document_title="Stable ABI",
        watermark=fullbleed.WatermarkSpec("text", "DRAFT"),
    )
    engine.register_bundle(bundle)
    html = "<main><p>Facade</p></main>"
    css = "p { color: #123456; }"
    rendered = engine.render_pdf(html, css)
    assert rendered.startswith(b"%PDF-")
    assert engine.document_metadata()["document_title"] == "Stable ABI"

    compiled = engine.compile_pdf(html, css)
    assert isinstance(compiled, fullbleed.CompiledDocument)
    assert type(compiled._handle).__name__ == "PyCapsule"
    assert compiled.render_pdf() == rendered
    assert compiled.stats()["page_count"] == 1
    assert compiled.stats()["command_count"] > 0
    assert compiled.render_pdf_batch(2).count(b"/Type /Page ") == 2


def test_compiled_document_renders_distinct_columnar_bindings(tmp_path) -> None:
    engine = fullbleed.PdfEngine()
    compiled = engine.compile_pdf(
        "<main><h1>Invoice</h1>"
        "<p>Invoice: {{invoice_id}}</p>"
        "<p>Customer: {{customer}}</p>"
        "<p>Amount: {{amount}}</p></main>",
        "body { font-family: Helvetica, sans-serif; font-size: 12pt; }",
    )
    stats = compiled.stats()
    assert stats["binding_slots"] == ["amount", "customer", "invoice_id"]
    assert stats["binding_program_page_count"] == 1
    assert stats["binding_program_command_count"] > 0
    bindings = {
        "invoice_id": ["INV-0001", "INV-0002", "INV-0003"],
        "customer": ["Ada Lovelace", "Grace Hopper", "Katherine Johnson"],
        "amount": ["$101.25", "$202.50", "$303.75"],
    }

    pdf = compiled.render_pdf_bindings(bindings)
    assert pdf.count(b"/Type /Page ") == 3
    assert b"INV-0001" in pdf
    assert b"INV-0002" in pdf
    assert b"INV-0003" in pdf
    assert b"Ada Lovelace" in pdf
    assert b"Katherine Johnson" in pdf
    assert b"{{" not in pdf

    output = tmp_path / "bound.pdf"
    digest = tmp_path / "bound.sha256"
    written = compiled.render_pdf_bindings_to_file(bindings, str(output), str(digest))
    assert written == output.stat().st_size
    assert output.read_bytes() == pdf
    assert len(digest.read_text(encoding="utf-8").strip()) == 64

    with pytest.raises(ValueError, match="binding columns do not match compiled slots"):
        compiled.render_pdf_bindings({"invoice_id": ["INV-ONLY"]})


def test_capsule_reentry_raises_instead_of_aliasing_native_state(monkeypatch) -> None:
    bundle = fullbleed.AssetBundle()

    class ReentrantResponse:
        def read(self) -> bytes:
            bundle.css()
            return b"unused"

    monkeypatch.setattr(urllib.request, "urlopen", lambda _url: ReentrantResponse())
    with pytest.raises(RuntimeError, match="already in use by another call"):
        bundle.add_file(
            "https://example.invalid/reentrant.bin",
            fullbleed.AssetKind.Other,
            remote=True,
        )


def test_dispatch_errors_are_restored_as_python_exceptions() -> None:
    with pytest.raises(ValueError, match="unknown FullBleed native operation"):
        native._dispatch(None, "not.an.operation", ())
    with pytest.raises(TypeError, match="expected 3 arguments"):
        native._dispatch(None, "build_features")
    with pytest.raises(TypeError, match="watermark must be a WatermarkSpec"):
        fullbleed.PdfEngine(watermark={"kind": "text", "value": "DRAFT"})


def test_stable_abi_module_can_initialize_in_a_subinterpreter() -> None:
    interpreters = pytest.importorskip("_xxsubinterpreters")
    identifier = interpreters.create()
    try:
        interpreters.run_string(
            identifier,
            "import sys; "
            "sys.meta_path[:] = [finder for finder in sys.meta_path "
            "if not getattr(finder, '_fullbleed_editable_finder', False)]; "
            "import fullbleed; "
            "assert fullbleed.build_features()['python']; "
            "assert fullbleed.PdfEngine().render_pdf('<p>sub</p>', '').startswith(b'%PDF-')",
        )
    finally:
        interpreters.destroy(identifier)
