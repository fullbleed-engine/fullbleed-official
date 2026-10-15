from __future__ import annotations

import pytest

from fullbleed.ui import (
    Document, Fragment, Heading, LayoutGrid, Paragraph, Row, Stack, Strong,
    StyleSheet, Text, component_styles, to_html,
)


def test_named_components_support_rich_text_and_conditional_fragments():
    html = to_html(Fragment(
        Heading("Hello, ", Strong("Alice & Bob"), level=1),
        Paragraph("Balance: ", Strong("$40"), "."),
        False,
    ))
    assert html == '<h1 class="ui-heading">Hello, <strong>Alice &amp; Bob</strong></h1><p>Balance: <strong>$40</strong>.</p>'


def test_stylesheet_snapshots_inputs_and_preserves_cascade_order():
    rules = {"@page": {"size": "A4", "margin": "18mm"}, ".amount": {"color": "red"}}
    base = StyleSheet(rules)
    final = StyleSheet(base, {".amount": {"color": "blue", "font_weight": 700}})
    rules[".amount"]["color"] = "green"
    assert base.to_css() == "@page { size: A4; margin: 18mm; }\n.amount { color: red; }"
    assert final.to_css().endswith('.amount { color: red; }\n.amount { color: blue; font-weight: 700; }')


def test_nested_at_rule_maps_fail_clearly():
    with pytest.raises(TypeError, match="Nested"):
        StyleSheet({"@media print": {"p": {"color": "red"}}})


def test_layout_props_become_css_and_explicit_styles_override_them():
    row = Row(Text("A"), Text("B"), gap="8pt", justify="start", wrap=True, style={"gap": "12pt"})
    html = to_html(row)
    assert 'gap="' not in html and 'wrap="' not in html
    assert "display: flex" in html and "flex-wrap: wrap" in html
    assert "gap: 8pt" not in html and "gap: 12pt" in html
    assert "grid-template-columns: 1fr 1fr 1fr" in to_html(LayoutGrid(columns=3, gap="4pt"))
    assert "flex-direction: column" in to_html(Stack(gap="4pt"))


@pytest.mark.parametrize("value", [0, -1, True])
def test_invalid_grid_columns_rejected(value):
    with pytest.raises(ValueError):
        LayoutGrid(columns=value)


def test_layout_prop_types_do_not_silently_choose_a_different_layout():
    with pytest.raises(TypeError):
        LayoutGrid(columns=2.5)
    with pytest.raises(TypeError):
        Row(wrap="nowrap")


@pytest.mark.parametrize("value", [0, 7, True, 1.5])
def test_heading_requires_a_real_heading_level(value):
    with pytest.raises(ValueError):
        Heading("Invalid", level=value)


def test_document_pdf_preview_and_export_share_the_same_styles(tmp_path):
    sheet = StyleSheet({"@page": {"size": "A5"}, ".ui-row": {"gap": "12pt"}})
    @Document(title="Styled", styles=sheet)
    def Report():
        return Row(Text("Left"), Text("Right"))

    class Engine:
        def __init__(self):
            self.calls = []
        def render_pdf(self, html, css):
            self.calls.append((html, css))
            return b"PDF"
        def render_image_pages(self, html, css, *, dpi):
            self.calls.append((html, css))
            assert dpi == 96
            return [b"PNG"]

    report = Report()
    engine = Engine()
    assert report.render_pdf(engine) == b"PDF"
    assert report.render_image_pages(engine, dpi=96) == [b"PNG"]
    assert engine.calls[0] == engine.calls[1]
    assert engine.calls[0][1] == component_styles() + "\n" + sheet.to_css()
    exported = report.emit_artifacts(html_path=tmp_path / "report.html", css_path=tmp_path / "report.css")
    assert exported["css"] == engine.calls[0][1]
    assert 'href="report.css"' in exported["html"]
    report.render_pdf(engine, styles={".ui-row": {"gap": "20pt"}})
    assert engine.calls[-1][1].endswith(".ui-row { gap: 20pt; }")
    assert report.to_css() == engine.calls[0][1]


def test_layout_defaults_can_be_disabled_without_changing_legacy_html():
    @Document(styles={"p": {"color": "red"}}, include_component_styles=False)
    def Report():
        return Paragraph("Hello")
    report = Report()
    assert report.to_css() == "p { color: red; }"
    assert "<style" not in report.to_html()


def test_default_component_styles_can_be_exported_without_a_custom_sheet(tmp_path):
    @Document()
    def Report():
        return Row(Text("Left"), Text("Right"))
    report = Report()
    result = report.emit_artifacts(html_path=tmp_path / "report.html", css_path=tmp_path / "report.css")
    assert result["css"] == report.to_css() == component_styles()
