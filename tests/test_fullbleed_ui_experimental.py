from __future__ import annotations

from dataclasses import FrozenInstanceError
from decimal import Decimal
import inspect

import pytest

from fullbleed.ui.experimental import (
    AuthoringError, Box, Card, Column, DataTable, Document, Flow, Fragment,
    Heading, Node, Page, Paragraph, Renderer, Stack, Style, Theme, component,
    css, mm, pct, pt, style, styled, token,
)


def test_styled_component_moves_between_documents_without_registering_css():
    Label = styled(Paragraph, font_size=pt(8), color=token("muted"))

    @component
    def Field(label: str, value: str):
        return Stack(Label(label, part="label"), Paragraph(value, part="value"), gap=pt(4))

    shared = Field("Customer", "A & B")
    blue = Document(shared, theme=Theme({"muted": "blue"}))
    red = Document(shared, theme=Theme({"muted": "red"}))
    assert "color: blue;" in blue.to_css()
    assert "color: red;" in red.to_css()
    assert "color: red;" not in blue.to_css()
    assert "A &amp; B" in blue.to_html()
    assert inspect.signature(Field).parameters["label"].annotation == "str"
    assert any(source.path == ("Field", "label") for source in blue.compile().sources)


def test_named_parts_do_not_require_selectors_or_leak_into_nested_components():
    @component
    def Inner():
        return Paragraph("Inner", part="title", style=style(color="green"))

    @component
    def Panel():
        return Card(Heading("Outer", part="title"), Inner())

    panel = Panel().with_parts(title=style(color="red"))
    css_text = Document(panel).to_css()
    assert "color: red;" in css_text and "color: green;" in css_text
    with pytest.raises(AuthoringError, match="Unknown component parts"):
        Panel().with_parts(missing=style(color="blue"))


def test_root_styles_work_without_custom_prop_forwarding_and_are_immutable():
    @component
    def Notice(message):
        return Card(Paragraph(message), style=style(color="black"))

    original = Notice("Paid")
    custom = original.with_style(color="blue")
    assert "color: black;" in Document(original).to_css()
    assert "color: blue;" in Document(custom).to_css()
    with pytest.raises(AuthoringError, match="explicit container"):
        Fragment(Paragraph("One"), Paragraph("Two")).with_style(color="blue")


def test_a_component_delegating_its_root_keeps_the_roots_exposed_parts():
    @component
    def WorkTable(rows):
        return DataTable(rows, columns=[Column("Work", str)])

    table = WorkTable(["First"]).with_parts(cell=style(color="blue"))
    assert "color: blue;" in Document(table).to_css()


def test_declared_parts_are_stable_across_conditional_content():
    @component(parts=("label", "note"))
    def Notice(message: str, note: str | None = None) -> Node:
        return Card(Paragraph(message, part="label"), note and Paragraph(note, part="note"))

    assert Notice.parts == ("label", "note")
    assert inspect.signature(Notice).parameters["message"].annotation == "str"
    empty, full = Notice("Paid"), Notice("Paid", "Thank you")
    assert empty.parts == full.parts == Notice.parts
    override = style(color="purple")
    assert "color: purple;" not in Document(empty.with_parts(note=override)).to_css()
    assert "color: purple;" in Document(full.with_parts(note=override)).to_css()
    assert "color: purple;" not in Document(full).to_css()
    with pytest.raises(AuthoringError, match="Available parts: label, note") as caught:
        empty.with_parts(notte=override)
    assert caught.value.path == ("Notice",)


def test_table_customization_does_not_depend_on_whether_rows_or_caption_exist():
    overrides = dict(cell=style(color="blue"), empty=style(color="gray"), caption=style(font_weight=700))
    empty = DataTable([], columns=[Column("Item", str)]).with_parts(**overrides)
    populated = DataTable(["One"], columns=[Column("Item", str)], caption="Items").with_parts(**overrides)
    assert empty.parts == populated.parts == DataTable.parts == ("header", "cell", "row", "caption", "empty")
    assert "color: gray;" in Document(empty).to_css()
    assert "color: blue;" in Document(populated).to_css()


def test_declared_parts_do_not_leak_through_nested_component_boundaries():
    @component(parts=("title", "note"))
    def Inner():
        return Card(Paragraph("Inner", part="title", style=style(color="green")))

    @component(parts=("title", "note"))
    def Outer():
        return Card(Paragraph("Outer", part="title"), Inner())

    result = Outer().with_parts(title=style(color="red"), note=style(color="purple"))
    css_text = Document(result).to_css()
    assert "color: red;" in css_text and "color: green;" in css_text
    assert "color: purple;" not in css_text


def test_root_delegation_preserves_optional_parts_and_can_expose_a_subset():
    @component
    def WorkTable(rows):
        return DataTable(rows, columns=[Column("Work", str)])

    @component(parts=("header",))
    def HeaderOnly(rows):
        return WorkTable(rows)

    assert WorkTable([]).parts == DataTable.parts
    WorkTable([]).with_parts(cell=style(color="blue"))
    restricted = HeaderOnly(["One"])
    assert restricted.parts == ("header",)
    assert "color: blue;" in Document(restricted.with_parts(header=style(color="blue"))).to_css()
    with pytest.raises(AuthoringError):
        restricted.with_parts(cell=style(color="blue"))


def test_part_declarations_are_captured_once_and_check_authored_markers():
    names = ["label", "note"]

    @component(parts=iter(names))
    def Notice(typo=False):
        return Paragraph("Paid", part="lable" if typo else "label")

    names.clear()
    assert Notice.parts == Notice().parts == ("label", "note")
    with pytest.raises(AuthoringError, match="missing from the component declaration: lable") as caught:
        Notice(typo=True)
    assert caught.value.path == ("Notice",)

    @component(parts=())
    def NoParts():
        return Paragraph("Paid")

    assert NoParts().parts == ()
    with pytest.raises(AuthoringError):
        NoParts().with_parts(label=style(color="blue"))


@pytest.mark.parametrize("parts", ["label", {"label"}, {"label": None}, ["label", "label"], [""], [1], ["bad name"]])
def test_invalid_part_declarations_fail_at_decoration(parts):
    with pytest.raises(AuthoringError):
        component(parts=parts)


def test_style_precedence_preserves_shorthand_order_and_caller_override():
    Notice = styled(Card, padding=pt(12), color="green")
    node = Notice("Paid", style=style(padding_left=pt(24), color="blue"))
    declarations = node.style.resolve(Theme())
    assert declarations.index("padding: 12pt") < declarations.index("padding-left: 24pt")
    assert "color: blue;" in declarations and "color: green;" not in declarations
    overridden = node.with_style(padding=pt(6)).style.resolve(Theme())
    assert overridden.index("padding-left: 24pt") < overridden.index("padding: 6pt")


def test_style_and_theme_inputs_are_snapshotted_at_construction():
    declarations = {"color": "red", "padding": [pt(1), pt(2)]}
    node = Paragraph("One", style=declarations)
    values = {"ink": "black"}
    theme = Theme(values, text={"color": token("ink")})
    document = Document(node, theme=theme)
    before = document.compile()
    declarations["color"] = "blue"
    declarations["padding"].append(pt(3))
    values["ink"] = "purple"
    assert document.compile() == before
    with pytest.raises(FrozenInstanceError):
        node.style = style(color="blue")


def test_node_constructor_captures_mutable_inputs_and_validates_void_content():
    children = ["One"]
    attributes = [("title", "Hint")]
    node = Node("p", children, attributes, {"color": "red"})
    children.append("Two")
    attributes.append(("class", "changed"))
    assert "Two" not in Document(node).to_html()
    assert "changed" not in Document(node).to_html()
    with pytest.raises(AuthoringError, match="Void"):
        Node("img", ["lost"])


@pytest.mark.parametrize("attributes", [
    {"STYLE": "color: red"}, {"DATA-FB-ID": "authored"}, {"TITLE": "first", "title": "second"},
])
def test_html_attribute_case_cannot_override_compiler_owned_values(attributes):
    with pytest.raises(AuthoringError):
        Paragraph("Example", attrs=attributes)


@pytest.mark.parametrize("value", [float("nan"), float("inf"), float("-inf"), Decimal("NaN"), True, "12"])
def test_invalid_lengths_are_rejected(value):
    with pytest.raises(AuthoringError):
        pt(value)


def test_property_typo_and_missing_units_are_actionable():
    with pytest.raises(AuthoringError, match="did you mean 'font-size'"):
        style(font_szie=pt(18))
    with pytest.raises(AuthoringError, match="explicit unit"):
        style(font_size=18)
    with pytest.raises(AuthoringError, match="explicit unit"):
        Document(Paragraph("One", style=style(font_size=token("size"))), theme=Theme({"size": 18})).compile()


def test_component_call_error_retains_component_path_and_original_cause():
    @component
    def Field():
        return Paragraph("Oops", font_szie=18)

    @component
    def Invoice():
        return Field()

    with pytest.raises(AuthoringError) as caught:
        Invoice()
    assert caught.value.path == ("Invoice", "Field")
    assert "font_szie" in caught.value.message
    assert caught.value.__cause__ is not None


def test_theme_failure_points_to_authored_component_part():
    @component
    def Total():
        return Paragraph("$12", part="amount", style=style(color=token("accent")))

    with pytest.raises(AuthoringError) as caught:
        Document(Total()).compile()
    assert caught.value.to_dict() == {
        "code": "theme.missing", "message": "Missing theme token 'accent'", "path": ["Total", "amount"],
    }


def test_css_escape_hatch_keeps_explicit_engine_declarations():
    value = style(css('transform: rotate(2deg); content: "a;b";'), color="blue")
    assert 'content: "a;b";' in value.resolve(Theme())
    assert "transform: rotate(2deg);" in value.resolve(Theme())
    with pytest.raises(AuthoringError):
        css('color: red; content: "unterminated')


def test_compilation_is_document_local_repeatable_and_handles_deep_nesting():
    first = Document(Paragraph("One", style=style(color="red")))
    before = first.compile()
    Document(Paragraph("Other", style=style(color="blue"))).compile()
    assert first.compile() == before
    node = Paragraph("Deep")
    for _ in range(1500):
        node = Box(node)
    assert "Deep" in Document(node).to_html()
    deep_part = Box(node, part="root").with_parts(root=style(color="blue"))
    assert "color: blue;" in Document(deep_part).to_css()


def test_ordered_children_are_consumed_once_and_mutable_collections_are_captured():
    calls = []
    def children():
        for value in range(3):
            calls.append(value)
            yield Paragraph(value)
    node = Box(children(), False, None, 0)
    document = Document(node)
    assert calls == [0, 1, 2]
    assert document.compile() == document.compile()
    assert calls == [0, 1, 2]
    for invalid in ({"key": "value"}, {"unordered"}, b"bytes", lambda: "forgot"):
        with pytest.raises(AuthoringError):
            Box(invalid)


def test_table_owns_header_empty_state_and_single_evaluation_of_cells():
    calls = []
    def cell(row):
        calls.append(row)
        return Paragraph(row)
    columns = [Column("Description", cell), Column("Amount", lambda row: 0, align="right", width=mm(30))]
    rows = ["A & B", "Second"]
    table = DataTable(iter(rows), columns=columns, caption="Work").with_parts(
        header=style(color="navy"), cell=style(padding=pt(10)), row=style(background="#fafafa"))
    document = Document(table)
    before = document.compile()
    rows.append("Late")
    columns.clear()
    assert document.compile() == before
    assert calls == ["A & B", "Second"]
    assert before.html.count('scope="col"') == 2
    assert "A &amp; B" in before.html and "Late" not in before.html
    assert "padding: 10pt;" in before.css and "background: #fafafa;" in before.css
    assert any(source.path == ("DataTable", "row 2", "Description", "cell") for source in before.sources)
    empty = Document(DataTable([], columns=[Column("A", str), Column("B", str)], empty="Nothing yet")).to_html()
    assert 'colspan="2"' in empty and "Nothing yet" in empty


def test_table_callback_error_has_row_and_column_context():
    with pytest.raises(AuthoringError) as caught:
        DataTable([{"value": 1}, {}], columns=[Column("Amount", lambda row: row["value"])])
    assert caught.value.path == ("DataTable", "row 2", "Amount")
    cause = caught.value
    while cause.__cause__ is not None:
        cause = cause.__cause__
    assert isinstance(cause, KeyError)


@pytest.mark.parametrize("kwargs", [
    {"columns": []}, {"columns": ["bad"]}, {"columns": [Column("A", str)], "repeat_header": "yes"},
    {"columns": [Column("A", str)], "row_break": "never"},
])
def test_invalid_table_contracts_are_rejected(kwargs):
    with pytest.raises(AuthoringError):
        DataTable([], **kwargs)


def test_page_configuration_changes_the_compiled_geometry_in_one_place():
    portrait = Document(Paragraph("A"), page=Page("A5", margin=mm(12))).to_css()
    landscape = Document(Paragraph("A"), page=Page("LETTER", margin=(mm(10), mm(15)), landscape=True)).to_css()
    custom = Document(Paragraph("A"), page=Page((mm(100), mm(200)), margin=mm(0))).to_css()
    assert "@page { size: A5; margin: 12mm; }" in portrait
    assert "size: LETTER landscape; margin: 10mm 15mm;" in landscape
    assert "size: 100mm 200mm; margin: 0mm;" in custom
    with pytest.raises(AuthoringError):
        Page(margin=pct(10))
    with pytest.raises(AuthoringError):
        Page((mm(0), mm(200)))


def test_flow_keeps_block_flow_and_requires_real_blocks():
    flow = Flow(Paragraph("First"), Paragraph("Second"), gap=pt(14))
    assert "margin-top: 14pt;" in Document(flow).to_css()
    assert "display: flex" not in Document(flow).to_css()
    with pytest.raises(AuthoringError, match="wrap text"):
        Flow("bare text")


def test_renderer_applies_metadata_per_document_without_shared_engine_mutation(monkeypatch):
    import fullbleed
    calls = []
    class Engine:
        def __init__(self, **options):
            calls.append(options)
        def export_render_time_pagination_trace(self, html, css):
            return {"events": []}
        def render_pdf(self, html, css):
            return html.encode()
    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    fonts = ["project.ttf"]
    renderer = Renderer(font_files=fonts)
    fonts.append("later.ttf")
    Document(Paragraph("A"), title="First", lang="fr").render_pdf(renderer)
    Document(Paragraph("B"), title="Second", lang="en").render_pdf(renderer)
    assert calls == [
        {"font_files": ["project.ttf"], "document_title": "First", "document_lang": "fr"},
        {"font_files": ["project.ttf"], "document_title": "Second", "document_lang": "en"},
    ]
    with pytest.raises(AuthoringError, match="Document owns"):
        Renderer(margin=20)


def test_experimental_renderer_rejects_native_unsplittable_overflow(monkeypatch):
    import fullbleed
    class Engine:
        def __init__(self, **options):
            pass
        def export_render_time_pagination_trace(self, html, css):
            return {"events": [{"reason": "forced_unsplittable_full_frame", "page": 2}]}
        def render_pdf(self, html, css):
            pytest.fail("An overflowing document must not be returned")
    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("Huge"), title="Invoice").render_pdf(Renderer())
    assert caught.value.code == "layout.unsplittable"
    assert caught.value.path == ("Invoice", "page 2")


def test_export_links_actual_relative_stylesheet(tmp_path):
    document = Document(Paragraph("A"))
    document.emit_artifacts(html_path=tmp_path / "html/report.html", css_path=tmp_path / "styles/report.css")
    assert 'href="../styles/report.css"' in (tmp_path / "html/report.html").read_text()
    assert (tmp_path / "styles/report.css").read_text() == document.to_css()


@pytest.mark.parametrize("property_name", ["margin", "padding", "border_width", "gap"])
@pytest.mark.parametrize("value", [(pt(2), 4), [0, Decimal("1.5")]])
def test_length_shorthand_numbers_need_units(property_name, value):
    with pytest.raises(AuthoringError) as caught:
        style(**{property_name: value})
    assert caught.value.code == "style.unit"


@pytest.mark.parametrize("number", [8, 8.5, Decimal("1E+2"), "8"])
def test_length_shorthand_tokens_keep_unit_checks_and_component_paths(number):
    @component
    def Panel():
        return Card("Body", style=style(padding=(pt(4), token("spacing"))))

    with pytest.raises(AuthoringError) as caught:
        Document(Panel(), theme=Theme({"spacing": number})).compile()
    assert caught.value.code == "style.unit"
    assert caught.value.path == ("Panel", "section")
    valid = Document(Panel(), theme=Theme({"spacing": mm(2)})).to_css()
    assert "padding: 4pt 2mm;" in valid
    assert style(padding=(0, 0.0, Decimal("0"), pt(2))).resolve(Theme()) == "padding: 0 0.0 0 2pt;"


def test_styled_factories_expose_their_optional_part_contract():
    @component(parts=("label",))
    def Field(value: str):
        return Paragraph(value, part="label")

    StyledField = styled(Field, color="blue")
    StyledParagraph = styled(Paragraph, color="blue")
    assert StyledField.parts == Field.parts == ("label",)
    assert StyledParagraph.parts is None
    assert "color: red;" in Document(StyledField("value").with_parts(label=style(color="red"))).to_css()
