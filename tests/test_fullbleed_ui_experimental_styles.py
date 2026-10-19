from dataclasses import FrozenInstanceError
from html.parser import HTMLParser

import pytest

from fullbleed.ui import StyleSheet
from fullbleed.ui.experimental import (
    AuthoringError, Document, Flow, Node, Page, Paragraph, Theme, component,
    mm, pt, style, styled, token,
)


def paragraphs(source):
    attributes = []

    class Capture(HTMLParser):
        def handle_starttag(self, tag, attrs):
            if tag == "p":
                attributes.append(dict(attrs))

    Capture().feed(source.html)
    return attributes


def test_document_snapshots_existing_stylesheets_and_selector_mappings():
    rules = {".notice": {"color": "blue", "font_size": "12pt"}}
    sheet = StyleSheet(rules)
    first = Document(Paragraph("One", class_name="notice"), styles=sheet)
    second = Document(Paragraph("Two", class_name="notice"), styles=rules)
    original = first.compile()
    sheet.add({".notice": {"color": "red"}})
    rules[".notice"]["color"] = "green"
    assert first.compile() == original
    assert first.styles == second.styles == ".notice { color: blue; font-size: 12pt; }"
    assert first.to_css().endswith(first.styles)
    with pytest.raises(FrozenInstanceError):
        first.styles = ""


def test_defaults_and_explicit_choices_keep_separate_native_cascade_priority():
    Label = styled(Paragraph, color=token("unused"), font_size=pt(8))
    first = Label("One", class_name="notice", style=style(color="blue"))
    second = first.with_style(color="purple")
    source = Document(first, second, styles=".notice { color: green }").compile()
    values = paragraphs(source)
    assert [value["style"] for value in values] == ["color: blue;", "color: purple;"]
    assert values[0]["class"] == values[1]["class"]
    assert values[0]["class"].startswith("notice fbx-")
    assert "font-size: 8pt;" in source.css
    assert "color: blue" not in source.css and "color: purple" not in source.css
    assert first.required_tokens == ()  # An explicitly replaced token is unused.
    assert Document(first).compile() == Document(first).compile()


def test_stylesheet_providers_are_evaluated_once_and_must_return_text():
    calls = []

    class SharedSheet:
        def to_css(self):
            calls.append(True)
            return ".notice { color:blue }"

    document = Document(Paragraph("One"), styles=SharedSheet())
    assert document.compile() == document.compile()
    assert calls == [True]

    class InvalidSheet:
        def to_css(self):
            return {".notice": {"color": "blue"}}

    with pytest.raises(AuthoringError, match="must return CSS text"):
        Document(Paragraph("One"), styles=InvalidSheet())


def test_class_name_remains_a_label_and_does_not_import_styles():
    document = Document(Paragraph("One", class_name="notice"))
    assert paragraphs(document.compile())[0]["class"].startswith("notice ")
    assert ".notice" not in document.to_css()


@pytest.mark.parametrize("rules,property_name", [
    ("@page { size: A5 }", "size"),
    ("@page :first { margin: 0 !important }", "margin"),
    ("@media print { @page invoice { margin-left: 2mm } }", "margin-left"),
    ("@media screen { @page { margin-top: 1mm } }", "margin-top"),
    ("@layer pages { @page { page-orientation: rotate-left } }", "page-orientation"),
    (r"@\70 age { m\61 rgin:0 }", "margin"),
])
def test_page_geometry_has_one_owner_even_in_nested_or_inactive_rules(rules, property_name):
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("One"), page=Page("A5", margin=mm(12)), styles=rules).compile()
    assert caught.value.code == "styles.page"
    assert caught.value.path[-1] == property_name
    assert "Document(page=Page(...))" in str(caught.value)


def test_page_furniture_strings_comments_and_element_margins_are_allowed():
    rules = '''/* @page { size:A3; margin:0 } */
    .notice { margin: 2pt; --example: "@page {size:A4}" }
    @page { color:blue; @bottom-center {content:"Page"; margin-top:2pt} }
    '''
    document = Document(Paragraph("One"), page=Page("A5"), styles=rules)
    assert document.styles.strip() == rules.strip()
    assert document.to_css().startswith("@page { size: A5;")


@pytest.mark.parametrize("rules", ["p { color:blue", "/* unclosed", "@media print { p {color:blue} } }"])
def test_stylesheet_syntax_errors_have_an_authoring_path(rules):
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("One"), styles=rules).compile()
    assert caught.value.code == "styles.syntax"
    assert caught.value.path == ("Document", "styles")


@pytest.mark.parametrize("rules", [False, 0, [], object()])
def test_document_rejects_non_stylesheet_inputs(rules):
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("One"), styles=rules)
    assert caught.value.code == "styles.type"


def test_embedded_styles_cannot_bypass_document_stylesheet_ownership():
    @component
    def Notice():
        return Flow(Node("style", ("@page :first { margin:0 }",)), Paragraph("One"))

    with pytest.raises(AuthoringError) as caught:
        Document(Notice()).compile()
    assert caught.value.code == "structure.stylesheet"
    assert caught.value.path == ("Notice", "style")


def test_conditional_style_fragments_omit_false_and_none_but_preserve_zero_values():
    normal = style(color="blue", margin=pt(2))
    assert style(normal, False, None, margin=0).resolve(Theme()) == "color: blue; margin: 0;"
    for invalid in (True, 0, ""):
        with pytest.raises(AuthoringError, match="Compose Style"):
            style(normal, invalid)


def test_selector_rules_accept_the_same_style_values_and_discover_theme_requirements():
    shared = style(color=token("accent"), padding=pt(12))
    rules = {".notice": shared}
    document = Document(Paragraph("Paid", class_name="notice"), styles=rules)
    rules[".notice"] = style(color="red")
    assert document.required_tokens == ("accent",)
    with pytest.raises(AuthoringError) as caught:
        document.compile()
    assert caught.value.code == "theme.missing"
    assert caught.value.path == ("Document", "styles", ".notice")
    themed = Document(*document.children, styles={".notice": shared}, theme=Theme({"accent": "blue"}))
    assert themed.styles == ".notice { color: blue; padding: 12pt; }"


def test_invalid_selector_declarations_fail_instead_of_silently_losing_appearance():
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("One"), styles={".notice": object()})
    assert caught.value.code == "styles.type"
