from __future__ import annotations

import importlib.util
from pathlib import Path

from fullbleed.ui import Document, compile_document, el, mount_component_html, render_node, to_html
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
    assert 'data-fb-role="document-root"' in html
    assert "<main" in html


def test_scaffold_template_fb_ui_is_reexport_shim() -> None:
    shim_path = ROOT / "python" / "fullbleed_cli" / "scaffold_templates" / "init" / "components" / "fb_ui.py"
    spec = importlib.util.spec_from_file_location("fb_ui_shim_test", shim_path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    from fullbleed.ui import mount_component_html as package_mount_component_html

    assert module.mount_component_html is package_mount_component_html
