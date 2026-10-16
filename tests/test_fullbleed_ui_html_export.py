from dataclasses import replace
from html.parser import HTMLParser

import pytest

from fullbleed.ui.experimental import (
    AuthoringError, Document, Paragraph, Theme, component, pt, styled, to_html,
    to_legacy, token,
)
from fullbleed.ui.experimental._resources import Resource


class Markup(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.tags = []
        self.text = []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def handle_data(self, data):
        self.text.append(data)


def test_document_explicit_styled_and_unstyled_exports_preserve_content():
    Note = styled(Paragraph, padding=pt(8), color="#315d9b")
    document = Document(Note("A < B & C", class_name="notice").with_style(font_weight=700),
                        styles={".notice": {"background": "#eeeeee"}})
    styled_html = document.to_html(include_styles=True)
    plain_html = document.to_html(include_styles=False)
    assert "<style>" in styled_html
    assert "padding: 8pt;" in styled_html and ".notice" in styled_html
    assert '<meta name="viewport"' in styled_html
    plain = Markup(plain_html)
    assert not any(tag in {"style", "link"} or "style" in attrs for tag, attrs in plain.tags)
    assert [(tag, attrs["class"]) for tag, attrs in plain.tags if "class" in attrs] == [("p", "notice")]
    assert "A < B & C" in plain.text
    assert document.to_html() == document.compile().html
    assert "<style>" not in document.to_html()
    assert "font-weight: 700;" in document.to_html()


def test_unstyled_export_does_not_resolve_unused_style_dependencies():
    document = Document(Paragraph("Content").with_style(color=token("missing")),
                        theme=Theme(text={"font_size": token("size")}),
                        styles={".unused": {"color": "red"}})
    assert "Content" in document.to_html(include_styles=False)
    with pytest.raises(AuthoringError, match="Missing theme token"):
        document.to_html(include_styles=True)


def test_fragment_styles_do_not_inject_document_geometry_or_text_rules():
    Label = styled(Paragraph, color=token("accent"))
    node = Label("A label").with_style(font_size=pt(11))
    html = node.to_html(include_styles=True, theme=Theme({"accent": "#315d9b"},
                                                       text={"color": token("document_only")}))
    tags = [tag for tag, _ in Markup(html).tags]
    assert tags == ["style", "p"]
    assert "@page" not in html and "body {" not in html
    assert "color: #315d9b;" in html and "font-size: 11pt;" in html
    assert "<style>" not in node.to_html(include_styles=False)
    assert "style=" not in node.to_html(include_styles=False)
    assert to_html(node, include_styles=False) == node.to_html(include_styles=False)


def test_owned_resources_are_collected_once_from_nested_boundaries_in_order():
    base = Resource("kit/base", "css", ".card { padding: 12px }")
    override = Resource("kit/theme", "css", ".card { padding: 16px }")

    @component
    def Card(text):
        return replace(Paragraph(text, class_name="card"), _resources=(base, override))

    document = Document(Card("One"), Card("Two"))
    source = document.compile()
    assert source.css.count(base.text) == source.css.count(override.text) == 1
    assert source.css.index(base.text) < source.css.index(override.text)
    assert base.text in document.to_html(include_styles=True)
    assert base.text not in document.to_html(include_styles=False)
    assert source == document.compile()


def test_conflicting_resource_versions_fail_with_component_path():
    @component
    def Different():
        return replace(Paragraph("Two"), _resources=(Resource("kit", "css", "p { color: blue }"),))

    first = replace(Paragraph("One"), _resources=(Resource("kit", "css", "p { color: red }"),))
    with pytest.raises(AuthoringError) as caught:
        Document(first, Different()).compile()
    assert caught.value.code == "resource.conflict"
    assert caught.value.path == ("Different",)


def test_component_resources_do_not_support_javascript():
    with pytest.raises(AuthoringError, match="require CSS text"):
        Resource("kit/javascript", "js", "window.exampleReady = true;")


def test_raw_text_closing_tags_cannot_escape_embedded_styles():
    css = 'p::after { content: "</StYlE><img src=x onerror=bad()>"; }'
    node = replace(Paragraph("Safe"), _resources=(Resource("css", "css", css),))
    html = Document(node).to_html(include_styles=True)
    tags = [tag for tag, _ in Markup(html).tags]
    assert "img" not in tags
    assert tags.count("style") == 1 and "script" not in tags
    assert "<\\/StYlE>" in html


def test_export_does_not_create_a_pdf_engine(monkeypatch):
    import fullbleed

    def unexpected(*args, **kwargs):
        raise AssertionError("HTML export created a PDF engine")

    monkeypatch.setattr(fullbleed, "PdfEngine", unexpected)
    assert "Hello" in Document(Paragraph("Hello")).to_html(include_styles=True)


@pytest.mark.parametrize("options", [{"include_styles": "yes"}, {"include_styles": 1}])
def test_export_options_are_explicit_booleans(options):
    with pytest.raises(TypeError):
        Document(Paragraph("Hello")).to_html(**options)
    with pytest.raises(TypeError):
        Paragraph("Hello").to_html(**options)


def test_explicit_theme_cannot_replace_an_existing_document_theme():
    with pytest.raises(AuthoringError, match="document already owns"):
        to_html(Document(Paragraph("Hello")), theme=Theme())


def test_bridge_does_not_silently_drop_owned_stylesheets():
    node = replace(Paragraph("Hello"), _resources=(Resource("kit", "css", "p { color: red }"),))
    with pytest.raises(AuthoringError) as caught:
        to_legacy(node)
    assert caught.value.code == "migration.resources"


def test_fragment_roots_can_be_inserted_into_lists_and_tables_without_weakening_document_checks():
    from fullbleed.ui.experimental import Node
    item = Node("li", ("List item",))
    row = Node("tr", (Node("td", ("Cell",)),))
    assert item.to_html(include_styles=False).startswith("<li")
    assert row.to_html(include_styles=False).startswith("<tr")
    with pytest.raises(AuthoringError, match="requires a ol, ul parent"):
        Document(item).compile()
    with pytest.raises(AuthoringError, match="needs inline content"):
        Node("p", (item,)).to_html(include_styles=False)
