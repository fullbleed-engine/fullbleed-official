from __future__ import annotations

import inspect
import urllib.request

import pytest

import fullbleed
import fullbleed._fullbleed as native


PUBLIC_NATIVE_NAMES = {
    "PdfEngine",
    "CompiledDocument",
    "CompiledFlowCompression",
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
        "CompiledFlowCompression",
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
    assert str(
        inspect.signature(fullbleed.CompiledDocument.render_pdf_reflow_bindings)
    ) == ("(self, /, bindings, deterministic_hash=None, *, compression='throughput')")
    assert str(
        inspect.signature(fullbleed.CompiledDocument.render_pdf_reflow_bindings_to_file)
    ) == (
        "(self, /, bindings, path, deterministic_hash=None, *, "
        "compression='throughput')"
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
    assert fullbleed.CompiledFlowCompression.Throughput == "throughput"
    assert fullbleed.CompiledFlowCompression.Compact == "compact"
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.AssetKind()
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.Asset()
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.CompiledDocument()
    with pytest.raises(TypeError, match="No constructor defined"):
        fullbleed.CompiledFlowCompression()
    with pytest.raises(TypeError, match="not an acceptable base type"):

        class AssetSubclass(fullbleed.Asset):
            pass

    with pytest.raises(TypeError, match="kind must be a string"):
        fullbleed.WatermarkSpec(1, "DRAFT")
    with pytest.raises(TypeError, match="opacity must be a real number"):
        fullbleed.WatermarkSpec("text", "DRAFT", opacity="0.5")


def test_build_features_reports_compiled_reflow_capabilities() -> None:
    features = fullbleed.build_features()
    assert features["compiled_reflow"] is True
    assert features["compiled_flow_compression_modes"] == ["throughput", "compact"]


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
    assert stats["reflow_program_ready"] is True
    assert stats["reflow_binding_slots"] == ["amount", "customer", "invoice_id"]
    assert stats["reflow_binding_slot_count"] == 3
    assert stats["reflow_program_node_count"] > 0
    assert stats["reflow_program_binding_text_node_count"] == 3
    assert stats["reflow_program_html_binding_node_count"] == 0
    assert stats["reflow_program_error"] is None
    assert stats["reflow_compression_modes"] == ["throughput", "compact"]
    assert stats["reflow_default_compression"] == "throughput"
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


def test_compiled_document_reflows_columnar_bindings_and_streams_to_file(
    tmp_path,
) -> None:
    engine = fullbleed.PdfEngine()
    template = (
        "<!doctype html><html><body><main>"
        "<h1>{{record_id}}</h1>"
        '<div class="content">{{content}}</div>'
        "<p>END {{record_id}}</p>"
        "</main></body></html>"
    )
    css = """
    @page { size: 240pt 180pt; margin: 18pt; }
    body { margin: 0; font-family: Helvetica, sans-serif;
           font-size: 10pt; line-height: 12pt; }
    h1 { margin: 0 0 6pt; font-size: 14pt; line-height: 16pt; }
    .content { white-space: pre-wrap; }
    p { margin: 6pt 0 0; }
    """
    compiled = engine.compile_pdf(template, css)
    bindings = {
        "record_id": ["REC-A", "REC-B", "REC-C"],
        "content": [
            "\n".join(f"alpha {index:03}" for index in range(3)),
            "\n".join(f"bravo {index:03}" for index in range(18)),
            "\n".join(f"charlie {index:03}" for index in range(35)),
        ],
    }

    first = compiled.render_pdf_reflow_bindings(bindings)
    second = compiled.render_pdf_reflow_bindings(bindings)
    assert first == second
    assert first.count(b"/Type /Page ") >= 6
    compact = compiled.render_pdf_reflow_bindings(
        bindings,
        compression=fullbleed.CompiledFlowCompression.Compact,
    )
    assert compact.count(b"/Type /Page ") == first.count(b"/Type /Page ")
    assert len(compact) <= len(first)
    assert compiled.render_pdf_reflow_bindings(bindings) == first
    with pytest.raises(ValueError, match="compression must be"):
        compiled.render_pdf_reflow_bindings(bindings, compression="maximum")
    with pytest.raises(TypeError, match="compression must be a string"):
        compiled.render_pdf_reflow_bindings(bindings, compression=64)

    output = tmp_path / "reflow-bindings.pdf"
    digest = tmp_path / "reflow-bindings.sha256"
    written = compiled.render_pdf_reflow_bindings_to_file(
        bindings, str(output), str(digest)
    )
    assert written == output.stat().st_size
    assert output.read_bytes() == first
    assert len(digest.read_text(encoding="utf-8").strip()) == 64
    extracted = fullbleed.extract_pdf_page_texts(str(output))
    assert extracted["ok"] is True
    text = "\n".join(page["text"] or "" for page in extracted["pages"])
    assert text.index("REC-A") < text.index("REC-B") < text.index("REC-C")
    assert "charlie 034" in text

    one = {
        "record_id": ["REC-ONE"],
        "content": ["\n".join(f"single {index:03}" for index in range(14))],
    }
    assert compiled.render_pdf_reflow_bindings(one) == engine.render_pdf(
        template.replace("{{record_id}}", "REC-ONE").replace(
            "{{content}}", one["content"][0]
        ),
        css,
    )

    with pytest.raises(ValueError, match="binding columns do not match compiled slots"):
        compiled.render_pdf_reflow_bindings({"record_id": ["REC-ONLY"]})


def test_compiled_reflow_supports_trusted_html_container_bindings(tmp_path) -> None:
    engine = fullbleed.PdfEngine()
    template = (
        "<main><h1>{{record_id}}</h1>"
        '<section data-fb-bind-html="sections"></section>'
        "<table><thead><tr><th>Row</th></tr></thead>"
        '<tbody data-fb-bind-html="rows"></tbody></table>'
        "<p>END {{record_id}}</p></main>"
    )
    css = """
    @page { size: 240pt 180pt; margin: 18pt; }
    body { margin: 0; font: 9pt/11pt Helvetica, sans-serif; }
    h1, p { margin: 0 0 5pt; }
    table { width: 100%; border-collapse: collapse; }
    th, td { border: 1pt solid #999; padding: 3pt; }
    tr { break-inside: avoid; }
    """
    compiled = engine.compile_pdf(template, css)
    stats = compiled.stats()
    assert stats["reflow_program_ready"] is True
    assert stats["reflow_program_html_binding_node_count"] == 2
    assert stats["reflow_binding_slots"] == ["record_id", "rows", "sections"]
    bindings = {
        "record_id": ["HTML-A", "HTML-B"],
        "sections": [
            "<p>HTML-A-P-000</p>",
            "".join(f"<p>HTML-B-P-{index:03}</p>" for index in range(14)),
        ],
        "rows": [
            "<tr><td>HTML-A-R-000</td></tr>",
            "".join(f"<tr><td>HTML-B-R-{index:03}</td></tr>" for index in range(26)),
        ],
    }
    output = tmp_path / "trusted-html-reflow.pdf"
    compiled.render_pdf_reflow_bindings_to_file(bindings, str(output))
    extracted = fullbleed.extract_pdf_page_texts(str(output))
    text = "\n".join(page["text"] or "" for page in extracted["pages"])
    assert text.index("HTML-A") < text.index("HTML-B")
    assert "HTML-B-P-013" in text
    assert "HTML-B-R-025" in text


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
