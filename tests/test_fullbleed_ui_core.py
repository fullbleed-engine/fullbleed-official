from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

from fullbleed.ui import (
    Document,
    compile_document,
    el,
    mount_component_html,
    render_node,
    to_html,
    validate_component_mount,
)
from fullbleed.ui.primitives import Box, Spacer, Text


FIXTURE_DIR = Path(__file__).parent / "fixtures" / "fullbleed_ui"
ROOT = Path(__file__).resolve().parents[1]


def _fixture(name: str) -> str:
    return (FIXTURE_DIR / name).read_text(encoding="utf-8")


def test_render_node_attr_normalization_snapshot() -> None:
    node = el(
        "section",
        "Hello <world>",
        el("span", 'Q"uote', class_name="child"),
        class_name="alpha beta",
        data_fb_role="demo",
        aria_hidden="true",
        hidden=True,
        disabled=False,
        title='5 > 4 "yes"',
    )
    assert render_node(node) == _fixture("render_node_attr_normalization.html")


def test_document_compile_snapshot() -> None:
    @Document(title="Snapshot <Doc>", bootstrap=False)
    def app() -> object:
        return Box(
            Text("Hello & hi", tag="p", class_name="copy"),
            Spacer(block="1rem", inline="2rem"),
        )

    assert compile_document(app()) == _fixture("document_compile.html")


def test_mount_component_html_passes_props_to_callable() -> None:
    def app(props: dict[str, str]) -> object:
        return el("div", props["message"], class_name="payload")

    html = mount_component_html(app, props={"message": "ok"})
    assert '<div class="payload">ok</div>' in html


def test_to_html_dispatches_for_element_and_document() -> None:
    node = el("div", "hello")
    assert to_html(node) == "<div>hello</div>"

    @Document(title="Dispatch", bootstrap=False)
    def app() -> object:
        return el("p", "x")

    artifact = app()
    assert to_html(artifact) == compile_document(artifact)
    assert artifact.to_html() == compile_document(artifact)
    assert node.to_html() == "<div>hello</div>"


def test_document_artifact_emit_artifacts_writes_html_css_with_doc_semantics(tmp_path: Path) -> None:
    @Document(title='Emit "Doc" <A&B>', bootstrap=False, lang="en-US")
    def app() -> object:
        return el("p", "payload")

    artifact = app()
    html_path = tmp_path / "out" / "doc.html"
    css_path = tmp_path / "out" / "doc.css"
    css_text = "@page { size: letter; }\nbody { color: #111; }"

    result = artifact.emit_artifacts(
        css=css_text,
        html_path=html_path,
        css_path=css_path,
        a11y_mode="raise",
    )

    html = html_path.read_text(encoding="utf-8")
    css = css_path.read_text(encoding="utf-8")

    assert result["html_path"] == str(html_path)
    assert result["css_path"] == str(css_path)
    assert css == css_text
    assert '<html lang="en-US">' in html
    assert "<title>Emit &quot;Doc&quot; &lt;A&amp;B&gt;</title>" in html
    assert 'rel="stylesheet"' in html
    assert 'href="doc.css"' in html
    assert 'data-fb-role="document-root"' in html
    assert "<main" in html
    assert result["document_css_href"] == "doc.css"
    assert result["document_css_media"] == "all"


def test_document_css_metadata_is_compiled_into_head() -> None:
    @Document(
        title="CSS Meta",
        bootstrap=False,
        css_href="styles/report.css",
        css_media="print",
    )
    def app() -> object:
        return el("p", "payload")

    html = compile_document(app())
    assert '<link rel="stylesheet" href="styles/report.css" media="print" />' in html


def test_document_emit_artifacts_reads_css_from_metadata_source_path(tmp_path: Path) -> None:
    css_src = tmp_path / "styles" / "report.css"
    css_src.parent.mkdir(parents=True, exist_ok=True)
    css_src.write_text("body{color:#123;}", encoding="utf-8")

    @Document(
        title="CSS Source",
        bootstrap=False,
        css_href="assets/report.css",
        css_source_path=str(css_src),
        css_media="all",
    )
    def app() -> object:
        return el("p", "payload")

    artifact = app()
    out_html = tmp_path / "out" / "doc.html"
    out_css = tmp_path / "out" / "doc.css"

    result = artifact.emit_artifacts(
        html_path=out_html,
        css_path=out_css,
        css=None,
    )
    html = out_html.read_text(encoding="utf-8")
    css = out_css.read_text(encoding="utf-8")
    assert css == "body{color:#123;}"
    assert 'href="assets/report.css"' in html
    assert result["document_css_href"] == "assets/report.css"


def test_document_css_required_raises_without_href(tmp_path: Path) -> None:
    @Document(title="CSS Required", bootstrap=False, css_required=True)
    def app() -> object:
        return el("p", "payload")

    artifact = app()
    with pytest.raises(ValueError):
        artifact.emit_html(tmp_path / "doc.html")


def test_scaffold_template_fb_ui_is_reexport_shim() -> None:
    shim_path = ROOT / "python" / "fullbleed_cli" / "scaffold_templates" / "init" / "components" / "fb_ui.py"
    spec = importlib.util.spec_from_file_location("fb_ui_shim_test", shim_path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    from fullbleed.ui import mount_component_html as package_mount_component_html

    assert module.mount_component_html is package_mount_component_html


def test_validate_component_mount_fails_on_text_overlap_from_render_trace() -> None:
    class FakeEngine:
        def render_pdf_with_glyph_report(self, html: str, css: str) -> tuple[bytes, list[object]]:
            return (b"%PDF-FAKE", [])

        def export_render_time_reading_order_trace(self, html: str, css: str) -> dict[str, object]:
            return {
                "schema": "fullbleed.pdf.reading_order_trace.v1",
                "pages": [
                    {
                        "page": 1,
                        "blocks": [
                            {
                                "index": 0,
                                "command_index": 10,
                                "kind": "draw_string",
                                "text": "left",
                                "bbox": {"x": 10, "y": 10, "w": 100, "h": 20},
                            },
                            {
                                "index": 1,
                                "command_index": 11,
                                "kind": "draw_string",
                                "text": "right",
                                "bbox": {"x": 80, "y": 12, "w": 120, "h": 18},
                            },
                        ],
                    }
                ],
            }

    report = validate_component_mount(
        engine=FakeEngine(),
        node_or_component=el("div", "x"),
        fail_on_overflow=True,
    )

    assert report["schema"] == "fullbleed.component_mount_validation.v2"
    assert report["schema_version"] == 2
    assert report["ok"] is False
    assert report["text_overlap_count"] == 1
    assert report["render_time_trace_available"] is True
    assert report["render_time_trace_error_present"] is False
    assert "debug_log" not in report
    assert "render_time_trace_error" not in report
    assert report["debug"]["debug_log_supplied"] is False
    assert report["debug"]["render_time_trace_error"] is None
    assert any(f["code"] == "TEXT_OVERLAP" for f in report["failures"])


def test_validate_component_mount_reports_asset_resolution_failures() -> None:
    class FakeEngine:
        def render_pdf_with_glyph_report(self, html: str, css: str) -> tuple[bytes, list[object]]:
            return (b"%PDF-FAKE", [])

        def export_render_time_asset_resolution_trace(self, html: str, css: str) -> dict[str, object]:
            return {
                "schema": "fullbleed.asset_resolution_trace.v1",
                "summary": {
                    "image_reference_count": 1,
                    "unresolved_count": 1,
                    "unsupported_count": 0,
                },
                "warnings": ["unresolved image source: missing-image.png"],
            }

    report = validate_component_mount(
        engine=FakeEngine(),
        node_or_component=el("img", src="missing-image.png", alt="Missing image"),
    )

    assert report["ok"] is False
    assert report["asset_reference_count"] == 1
    assert report["asset_unresolved_count"] == 1
    assert report["asset_unsupported_count"] == 0
    assert any(f["code"] == "ASSET_RESOLUTION" for f in report["failures"])
    assert report["debug"]["asset_resolution_trace_error"] is None


def test_validate_component_mount_uses_native_pagination_trace_for_overflow_and_overprint() -> None:
    class FakeEngine:
        def render_pdf_with_glyph_report(self, html: str, css: str) -> tuple[bytes, list[object]]:
            return (b"%PDF-FAKE", [])

        def export_render_time_pagination_trace(self, html: str, css: str) -> dict[str, object]:
            return {
                "schema": "fullbleed.pagination_trace.v1",
                "summary": {
                    "event_count": 2,
                    "overflow_event_count": 1,
                    "flowable_overlap_count": 1,
                    "text_overlap_count": 1,
                    "low_coverage_page_count": 0,
                },
                "events": [
                    {
                        "event_type": "layout",
                        "page": 1,
                        "result": "overflow",
                        "flowable_name": "Paragraph",
                        "frame_index": 0,
                        "reason": "frame_overflow",
                        "overflow_severity": "frame_advance",
                    }
                ],
                "pages": [
                    {
                        "page": 1,
                        "flowable_overlap_count": 1,
                        "text_overlap_count": 1,
                        "text_overlap_samples": [
                            {
                                "page": 1,
                                "overlap_bbox": {"x": 12, "y": 14, "w": 20, "h": 8},
                                "a": {
                                    "index": 0,
                                    "command_index": 10,
                                    "top_role": "P",
                                    "text": "left",
                                },
                                "b": {
                                    "index": 1,
                                    "command_index": 11,
                                    "top_role": "P",
                                    "text": "right",
                                },
                            }
                        ],
                        "occupied_area_ratio": 0.8,
                        "low_coverage": False,
                    }
                ],
            }

    report = validate_component_mount(
        engine=FakeEngine(),
        node_or_component=el("div", "x"),
        fail_on_overflow=True,
    )

    assert report["ok"] is False
    assert report["overflow_count"] == 1
    assert report["flowable_overlap_count"] == 1
    assert report["text_overlap_count"] == 1
    assert report["pagination_trace_available"] is True
    assert report["pagination_trace_event_count"] == 2
    assert report["pagination_trace_error_present"] is False
    assert any(f["code"] == "OVERFLOW" for f in report["failures"])
    assert any(f["code"] == "TEXT_OVERLAP" for f in report["failures"])
    assert any(f["code"] == "FLOWABLE_OVERPRINT" for f in report["failures"])
    assert report["debug"]["pagination_trace_error"] is None
