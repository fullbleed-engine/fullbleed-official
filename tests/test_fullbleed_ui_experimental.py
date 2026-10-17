from __future__ import annotations

from dataclasses import FrozenInstanceError
from decimal import Decimal
import inspect
from html.parser import HTMLParser

import pytest

from fullbleed.ui.experimental import (
    AuthoringError, Box, Card, Column, DataTable, Document, DocumentSource, Flow, Fragment, Grid,
    Heading, LayoutError, Node, Page, Paragraph, Renderer, Row, Stack, Style, Theme, component,
    css, mm, pct, pt, style, styled, to_legacy, token,
)


def _styles(document_or_source):
    """Inspect both stylesheet defaults and explicit inline declarations."""
    source = document_or_source.compile() if isinstance(document_or_source, Document) else document_or_source
    declarations = [source.css]

    class Styles(HTMLParser):
        def handle_starttag(self, tag, attrs):
            declarations.extend(value for key, value in attrs if key == "style")

    Styles().feed(source.html)
    return "\n".join(declarations)


def test_document_source_distinguishes_serialized_input_from_native_layout():
    document = Document(Paragraph("Hello"))
    source = document.compile(a11y_mode="raise")
    assert isinstance(source, DocumentSource)
    assert type(source).__name__ == "DocumentSource"
    assert not hasattr(source, "render_pdf")
    assert source.a11y_report["ok"] is True
    assert source.html == document.to_html()
    assert source.css == document.to_css()
    assert document.compile().a11y_report is None


@component
def _DuplicateIds():
    return Flow(Paragraph("One", attrs={"id": "same"}, part="first"),
                Paragraph("Two", attrs={"id": "same"}, part="second"))


def test_semantic_report_preserves_shared_rules_and_attaches_authored_locations():
    from fullbleed.ui.accessibility import A11yContract, A11yWarning

    node = _DuplicateIds()
    legacy = A11yContract().validate(to_legacy(node), mode=None)
    with pytest.warns(A11yWarning, match="_DuplicateIds -> second"):
        source = Document(node).compile(a11y_mode="warn")
    report = source.a11y_report
    assert report["error_count"] == legacy["error_count"] == 1
    assert report["warning_count"] == legacy["warning_count"] == 0
    diagnostic = report["diagnostics"][0]
    assert diagnostic["code"] == legacy["diagnostics"][0]["code"] == "ID_DUPLICATE"
    assert diagnostic["source"]["path"] == ["_DuplicateIds", "second"]
    assert diagnostic["source"]["file"].endswith("test_fullbleed_ui_experimental.py")
    assert diagnostic["source"]["line"] > 0
    assert diagnostic["related_sources"][0]["path"] == ["_DuplicateIds", "first"]
    report["diagnostics"].clear()
    assert len(source.a11y_report["diagnostics"]) == 1
    assert "test_fullbleed_ui_experimental.py" not in source.html


@pytest.mark.parametrize("operation", ["compile", "to_html", "to_css", "prepare", "render_pdf", "render_image_pages", "emit_artifacts"])
def test_strict_semantics_reject_before_rendering_or_writing(operation, tmp_path):
    from fullbleed.ui.accessibility import A11yValidationError

    class NoRender:
        def engine_for(self, document):
            pytest.fail("Semantic failures must be reported before creating a renderer")

    document = Document(_DuplicateIds())
    method = getattr(document, operation)
    with pytest.raises(A11yValidationError) as caught:
        if operation in {"prepare", "render_pdf", "render_image_pages"}:
            method(NoRender(), a11y_mode="raise")
        elif operation == "emit_artifacts":
            method(html_path=tmp_path / "invalid.html", css_path=tmp_path / "invalid.css", a11y_mode="raise")
        else:
            method(a11y_mode="raise")
    assert caught.value.report["diagnostics"][0]["source"]["path"] == ["_DuplicateIds", "second"]
    assert not list(tmp_path.iterdir())


@pytest.mark.parametrize("mode", [None, "", "none"])
def test_semantics_are_explicitly_optional(mode):
    source = Document(_DuplicateIds()).compile(a11y_mode=mode)
    assert source.a11y_report is None


def test_invalid_semantic_mode_is_not_silently_ignored():
    with pytest.raises(ValueError, match="Unsupported a11y_mode"):
        Document(Paragraph("Hello")).compile(a11y_mode="strict")


def test_content_model_rejects_nested_blocks_across_components_and_fragments():
    @component
    def Inner():
        return Paragraph("Inner")

    @component
    def Outer():
        return Paragraph("Outer", Fragment(Inner()))

    with pytest.raises(AuthoringError, match="inline content") as caught:
        Document(Outer()).compile()
    assert caught.value.code == "structure.child"
    assert caught.value.path == ("Outer", "Inner", "p")
    with pytest.raises(AuthoringError, match="inline content"):
        Document(Paragraph(Node("span", (Box("Wrong"),)))).to_html()


@pytest.mark.parametrize("node,code", [
    (Node("table", (Paragraph("Wrong"),)), "structure.child"),
    (Node("table", (Fragment("Wrong"),)), "structure.child"),
    (Node("tr", (Node("td", ("Wrong",)),)), "structure.parent"),
    (Node("td", ("Wrong",)), "structure.parent"),
    (Node("ul", (Paragraph("Wrong"),)), "structure.child"),
    (Node("a", (Fragment(Node("a", ("Wrong",))),)), "structure.link"),
])
def test_content_model_checks_table_list_and_link_boundaries(node, code):
    with pytest.raises(AuthoringError) as caught:
        Document(node).compile()
    assert caught.value.code == code


def test_content_model_accepts_rich_text_transparent_links_tables_and_svg():
    from fullbleed.ui.experimental import Strong, Text

    document = Document(
        Paragraph("Hello ", Fragment(Text("there"), None), Strong("!")),
        Node("a", (Paragraph("A whole linked block"),), attributes=(("href", "#target"),)),
        Node("table", ("\n", Fragment(Node("tbody", (Node("tr", (Node("td", ("Cell",)),)),))))),
        Node("ul", (Fragment(Node("li", ("Item",))),)),
        Paragraph(Node("svg", (Node("g", (Node("path"),)),), attributes=(("alt", "Mark"),))),
    )
    source = document.compile(a11y_mode="raise")
    assert source.a11y_report["ok"]
    assert "Cell" in source.html and "Item" in source.html and "<svg" in source.html


def test_strict_semantics_handle_deep_component_trees():
    from fullbleed.ui.experimental import Text

    node = Text("Deep heading")
    for _ in range(1500):
        node = Text(node)
    source = Document(Heading(node, level=1)).compile(a11y_mode="raise")
    assert source.a11y_report["ok"]


def test_styled_component_moves_between_documents_without_registering_css():
    Label = styled(Paragraph, font_size=pt(8), color=token("muted"))

    @component
    def Field(label: str, value: str):
        return Stack(Label(label, part="label"), Paragraph(value, part="value"), gap=pt(4))

    shared = Field("Customer", "A & B")
    blue = Document(shared, theme=Theme({"muted": "blue"}))
    red = Document(shared, theme=Theme({"muted": "red"}))
    assert "color: blue;" in _styles(blue)
    assert "color: red;" in _styles(red)
    assert "color: red;" not in _styles(blue)
    assert "A &amp; B" in blue.to_html()
    assert inspect.signature(Field).parameters["label"].annotation == "str"
    assert any(source.path == ("Field", "label") for source in blue.compile().sources)


def test_theme_updates_preserve_text_and_snapshot_values_without_changing_shared_documents():
    base = Theme({"accent": "blue", "spacing": pt(6)}, text=style(color=token("accent"), font_size=pt(10)))
    content = Paragraph("Shared", style=style(padding=token("spacing")))
    original = Document(content, theme=base)
    before = original.compile()
    updates = {"accent": "green", "spacing": pt(8)}
    warm = base.with_values(updates, accent="red", **{"brand.note": "Warm"})
    updates["spacing"] = pt(99)
    assert warm.text == base.text
    assert dict(warm.values) == {"accent": "red", "spacing": pt(8), "brand.note": "Warm"}
    assert dict(base.values) == {"accent": "blue", "spacing": pt(6)}
    assert original.compile() == before
    changed = _styles(Document(content, theme=warm))
    assert "color: red;" in changed and "padding: 8pt;" in changed and "font-size: 10pt;" in changed
    assert "padding: 99pt;" not in changed
    assert base.with_values() == base
    assert dict(base.with_values({"accent": "green"}, {"accent": "orange"}).values)["accent"] == "orange"


@pytest.mark.parametrize("value", [[], "", (), 0, False])
def test_theme_requires_mappings_for_construction_and_updates(value):
    with pytest.raises(AuthoringError) as constructor:
        Theme(value)
    with pytest.raises(AuthoringError) as update:
        Theme().with_values(value)
    assert constructor.value.code == update.value.code == "theme.values"


@pytest.mark.parametrize("updates,code", [
    ({"bad token": "red"}, "theme.name"),
    ({"accent": token("other")}, "theme.value"),
    ({"gap": [pt(1), pt(2)]}, "theme.value"),
    ({"gap": float("inf")}, "value.finite"),
    ({"accent": None}, "style.value"),
])
def test_theme_updates_use_constructor_validation(updates, code):
    with pytest.raises(AuthoringError) as caught:
        Theme().with_values(updates)
    assert caught.value.code == code


def test_token_discovery_uses_effective_styles_and_crosses_component_boundaries():
    calls = []
    Label = styled(Paragraph, color=token("discarded"), padding=(token("space"), pt(2)))

    @component(parts=("label",), optional_parts=("note",))
    def Notice(message, note=None):
        calls.append(message)
        return Flow(Label(message, part="label", style=style(color=token("ink"))),
                    Paragraph(note, part="note", style=style(color=token("muted"))) if note else None,
                    gap=0)

    shared = Notice("Hello")
    updated = shared.with_parts(label=style(color=token("accent")), note=style(color=token("optional")))
    assert shared.required_tokens == ("ink", "space")
    assert updated.required_tokens == ("accent", "space")
    assert Notice("With note", "Optional").required_tokens == ("ink", "muted", "space")
    themed = Theme({"unused": "green"}, text=style(color=token("body_ink")))
    doc = Document(Box(shared, updated, shared), theme=themed)
    assert doc.required_tokens == ("accent", "body_ink", "ink", "space")
    assert doc.required_tokens == doc.required_tokens
    assert calls == ["Hello", "With note"]
    # Discovery does not require a complete theme or suppress contextual errors.
    with pytest.raises(AuthoringError) as caught:
        doc.compile()
    assert caught.value.code == "theme.missing"
    assert caught.value.path == ("Notice", "label")
    complete = themed.with_values(ink="black", accent="red", body_ink="gray", space=pt(4))
    assert "color: red;" in _styles(Document(*doc.children, theme=complete))


def test_token_discovery_is_explicit_and_does_not_parse_css_variable_strings():
    node = Paragraph("Text", style=style(css("color: var(--brand)"), **{"--brand": token("accent")}))
    assert node.required_tokens == ("accent",)
    assert Document("Plain text").required_tokens == ()
    assert Fragment().required_tokens == ()


def test_named_parts_do_not_require_selectors_or_leak_into_nested_components():
    @component
    def Inner():
        return Paragraph("Inner", part="title", style=style(color="green"))

    @component
    def Panel():
        return Card(Heading("Outer", part="title"), Inner())

    panel = Panel().with_parts(title=style(color="red"))
    css_text = _styles(Document(panel))
    assert "color: red;" in css_text and "color: green;" in css_text
    with pytest.raises(AuthoringError, match="Unknown component parts"):
        Panel().with_parts(missing=style(color="blue"))


def test_root_styles_work_without_custom_prop_forwarding_and_are_immutable():
    @component
    def Notice(message):
        return Card(Paragraph(message), style=style(color="black"))

    original = Notice("Paid")
    custom = original.with_style(color="blue")
    assert "color: black;" in _styles(Document(original))
    assert "color: blue;" in _styles(Document(custom))
    with pytest.raises(AuthoringError, match="explicit container"):
        Fragment(Paragraph("One"), Paragraph("Two")).with_style(color="blue")


def test_a_component_delegating_its_root_keeps_the_roots_exposed_parts():
    @component
    def WorkTable(rows):
        return DataTable(rows, columns=[Column("Work", str)])

    table = WorkTable(["First"]).with_parts(cell=style(color="blue"))
    assert "color: blue;" in _styles(Document(table))


def test_declared_parts_are_stable_across_conditional_content():
    @component(parts=("label",), optional_parts=("note",))
    def Notice(message: str, note: str | None = None) -> Node:
        return Card(Paragraph(message, part="label"), note and Paragraph(note, part="note"))

    assert Notice.parts == ("label", "note")
    assert inspect.signature(Notice).parameters["message"].annotation == "str"
    empty, full = Notice("Paid"), Notice("Paid", "Thank you")
    assert empty.parts == full.parts == Notice.parts
    override = style(color="purple")
    assert "color: purple;" not in _styles(Document(empty.with_parts(note=override)))
    assert "color: purple;" in _styles(Document(full.with_parts(note=override)))
    assert "color: purple;" not in _styles(Document(full))
    with pytest.raises(AuthoringError, match="Available parts: label, note") as caught:
        empty.with_parts(notte=override)
    assert caught.value.path == ("Notice",)


def test_table_customization_does_not_depend_on_whether_rows_or_caption_exist():
    overrides = dict(cell=style(color="blue"), empty=style(color="gray"), caption=style(font_weight=700))
    empty = DataTable([], columns=[Column("Item", str)]).with_parts(**overrides)
    populated = DataTable(["One"], columns=[Column("Item", str)], caption="Items").with_parts(**overrides)
    assert empty.parts == populated.parts == DataTable.parts == ("header", "cell", "row", "caption", "empty")
    assert "color: gray;" in _styles(Document(empty))
    assert "color: blue;" in _styles(Document(populated))


def test_declared_parts_do_not_leak_through_nested_component_boundaries():
    @component(parts=("title",), optional_parts=("note",))
    def Inner():
        return Card(Paragraph("Inner", part="title", style=style(color="green")))

    @component(parts=("title",), optional_parts=("note",))
    def Outer():
        return Card(Paragraph("Outer", part="title"), Inner())

    result = Outer().with_parts(title=style(color="red"), note=style(color="purple"))
    css_text = _styles(Document(result))
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
    assert "color: blue;" in _styles(Document(restricted.with_parts(header=style(color="blue"))))
    with pytest.raises(AuthoringError):
        restricted.with_parts(cell=style(color="blue"))


def test_part_declarations_are_captured_once_and_check_authored_markers():
    names, optional = ["label"], ["note"]

    @component(parts=iter(names), optional_parts=iter(optional))
    def Notice(typo=False):
        return Paragraph("Paid", part="lable" if typo else "label")

    names.clear()
    optional.clear()
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
    declarations = _styles(Document(node))
    assert declarations.index("padding: 12pt") < declarations.index("padding-left: 24pt")
    assert "color: blue;" in declarations and "color: green;" not in declarations
    overridden = _styles(Document(node.with_style(padding=pt(6))))
    assert overridden.index("padding-left: 24pt") < overridden.index("padding: 6pt")


def test_styled_preserves_business_arguments_and_invokes_the_factory_once():
    calls = []

    @component(parts=("value",))
    def Choice(label: str, /, *, style: str) -> Node:
        calls.append((label, style))
        return Paragraph(label, ": ", style, part="value")

    StyledChoice = styled(Choice, color="red")
    node = StyledChoice("Invitation", style="formal")
    assert calls == [("Invitation", "formal")]
    assert inspect.signature(StyledChoice) == inspect.signature(Choice)
    assert StyledChoice.parts == ("value",)
    compiled = Document(node).compile()
    assert "Invitation: formal" in compiled.html and "color: red;" in compiled.css
    assert "color: blue;" in _styles(Document(node.with_style(color="blue")))
    assert "color: red;" in _styles(Document(node))
    assert calls == [("Invitation", "formal")]


def test_styled_layers_preserve_explicit_styles_and_nested_recipe_precedence():
    Base = styled(Card, padding=pt(18), color="green")
    Outer = styled(Base, padding_left=pt(24), color="red")
    node = Outer("Invoice", style=style(color="blue"))
    compiled = _styles(Document(node))
    assert compiled.index("padding: 18pt") < compiled.index("padding-left: 24pt")
    assert "color: blue;" in compiled and "color: green;" not in compiled and "color: red;" not in compiled
    assert "color: red;" in _styles(Document(Outer("Invoice")))

    @component
    def Explicit():
        return Paragraph("Important").with_style(color="purple")

    result = styled(Explicit, color="red", padding=pt(8))()
    assert "color: purple;" in _styles(Document(result))
    assert "padding: 8pt;" in _styles(Document(result))
    assert "color: blue;" in _styles(Document(result.with_style(color="blue")))
    legacy = to_legacy(result)
    assert "color: purple;" in legacy[0].props["style"]
    assert "padding: 8pt;" in legacy[0].props["style"]


def test_styled_no_longer_injects_style_into_custom_function_arguments():
    def Notice(message):
        return Paragraph(message)

    with pytest.raises(TypeError, match="unexpected keyword argument 'style'"):
        styled(Notice, color="red")("Paid", style=style(color="blue"))
    assert "color: blue;" in _styles(Document(styled(Notice, color="red")("Paid").with_style(color="blue")))


@pytest.mark.parametrize("factory,argument,property_name,builtin,builtin_css,recipe,given,given_css", [
    (Flow, "gap", "row-gap", pt(8), "8pt", pt(12), pt(3), "3pt"),
    (Stack, "gap", "gap", pt(8), "8pt", pt(12), 0, "0"),
    (Stack, "align", "align-items", "stretch", "stretch", "end", "start", "start"),
    (Row, "gap", "gap", pt(8), "8pt", pt(12), pt(3), "3pt"),
    (Row, "align", "align-items", "center", "center", "end", "start", "start"),
    (Row, "justify", "justify-content", "space-between", "space-between", "end", "start", "start"),
    (Grid, "gap", "gap", pt(8), "8pt", pt(12), pt(3), "3pt"),
    (Grid, "columns", "grid-template-columns", 2, "1fr 1fr", "3fr 1fr", 3, "1fr 1fr 1fr"),
    (Heading, "keep_with_next", "break-after", True, "avoid", "auto", True, "avoid"),
    (Heading, "keep_with_next", "break-after", True, "avoid", "avoid", False, "auto"),
])
def test_explicit_layout_choices_override_recipes_while_omitted_choices_do_not(
    factory, argument, property_name, builtin, builtin_css, recipe, given, given_css,
):
    child = Paragraph("Content") if factory is not Heading else "Content"
    recipe_factory = styled(factory, {property_name: recipe})

    def declaration(node, expected):
        assert f"{property_name}: {expected};" in _styles(Document(node))

    declaration(factory(child), builtin_css)
    declaration(factory(child, **{argument: None}), builtin_css)
    declaration(recipe_factory(child), str(recipe))
    declaration(recipe_factory(child, **{argument: None}), str(recipe))
    # Explicitly choosing the built-in value is different from omitting it.
    declaration(recipe_factory(child, **{argument: builtin}), builtin_css)
    chosen = recipe_factory(child, **{argument: given})
    declaration(chosen, given_css)
    declaration(styled(recipe_factory, {property_name: builtin_css})(child, **{argument: given}), given_css)
    declaration(recipe_factory(child, **{argument: given}, style={property_name: builtin_css}), builtin_css)
    declaration(chosen.with_style({property_name: recipe}), str(recipe))
    declaration(chosen, given_css)
    assert f"{property_name}: {given_css};" in to_legacy(chosen)[0].props["style"]


def test_component_authored_layout_choices_survive_outer_styling_and_optional_forwarding():
    @component
    def ChosenFlow():
        return Flow(Paragraph("Chosen"), gap=pt(3))

    @component
    def OptionalFlow(*, gap=None):
        return Flow(Paragraph("Optional"), gap=gap)

    for factory in (ChosenFlow, OptionalFlow):
        decorated = styled(factory, row_gap=pt(12))
        expected = "3pt" if factory is ChosenFlow else "12pt"
        assert f"row-gap: {expected};" in _styles(Document(decorated()))
    assert "row-gap: 0;" in _styles(Document(styled(OptionalFlow, row_gap=pt(12))(gap=0)))


@pytest.mark.parametrize("factory,kwargs", [
    (Flow, {"gap": False}), (Stack, {"align": "around"}),
    (Row, {"justify": False}), (Grid, {"columns": False}),
    (Grid, {"columns": 0}), (Heading, {"keep_with_next": 0}),
])
def test_optional_layout_choices_still_reject_invalid_explicit_values(factory, kwargs):
    with pytest.raises(AuthoringError):
        factory(Paragraph("Content"), **kwargs)


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
    assert "color: blue;" in _styles(Document(deep_part))


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
    assert "padding: 10pt;" in _styles(before) and "background: #fafafa;" in _styles(before)
    assert any(source.path == ("DataTable", "row 2", "column 1", "Description", "cell") for source in before.sources)
    empty = Document(DataTable([], columns=[Column("A", str), Column("B", str)], empty="Nothing yet")).to_html()
    assert 'colspan="2"' in empty and "Nothing yet" in empty


def test_table_callback_error_has_row_and_column_context():
    with pytest.raises(AuthoringError) as caught:
        DataTable([{"value": 1}, {}], columns=[Column("Amount", lambda row: row["value"])])
    assert caught.value.path == ("DataTable", "row 2", "column 1", "Amount")
    cause = caught.value
    while cause.__cause__ is not None:
        cause = cause.__cause__
    assert isinstance(cause, KeyError)


def test_duplicate_column_labels_have_distinct_header_cell_and_callback_locations():
    table = DataTable([1], columns=[Column("Value", str), Column("Value", str)])
    sources = Document(table).compile().sources
    for part, prefix in (("header", ("DataTable",)), ("cell", ("DataTable", "row 1"))):
        selected = [source for source in sources if source.path[-1] == part]
        assert [source.path for source in selected] == [(*prefix, f"column {i}", "Value", part) for i in (1, 2)]
        assert len({source.node_id for source in selected}) == 2
    with pytest.raises(AuthoringError) as caught:
        DataTable([{}], columns=[Column("Value", lambda row: "First"), Column("Value", lambda row: row["missing"])])
    assert caught.value.path == ("DataTable", "row 1", "column 2", "Value")
    assert caught.value.code == "table.cell"


def test_omitted_table_choices_and_explicit_none_compile_identically():
    omitted = DataTable(["One"], columns=[Column("Item", str)])
    forwarded = DataTable(["One"], columns=[Column("Item", str, align=None)], row_break=None)
    assert Document(omitted).compile() == Document(forwarded).compile()


@pytest.mark.parametrize("kwargs", [
    {"columns": []}, {"columns": ["bad"]}, {"columns": [Column("A", str)], "repeat_header": "yes"},
    {"columns": [Column("A", str)], "row_break": "never"},
])
def test_invalid_table_contracts_are_rejected(kwargs):
    with pytest.raises(AuthoringError):
        DataTable([], **kwargs)


def test_page_configuration_changes_the_compiled_geometry_in_one_place():
    portrait = _styles(Document(Paragraph("A"), page=Page("A5", margin=mm(12))))
    landscape = _styles(Document(Paragraph("A"), page=Page("LETTER", margin=(mm(10), mm(15)), landscape=True)))
    custom = _styles(Document(Paragraph("A"), page=Page((mm(100), mm(200)), margin=mm(0))))
    assert "@page { size: A5; margin: 12mm; }" in portrait
    assert "size: LETTER landscape; margin: 10mm 15mm;" in landscape
    assert "size: 100mm 200mm; margin: 0mm;" in custom
    with pytest.raises(AuthoringError):
        Page(margin=pct(10))
    with pytest.raises(AuthoringError):
        Page((mm(0), mm(200)))


def test_flow_keeps_block_flow_and_requires_real_blocks():
    child = Paragraph("Second").with_style(margin_top=pt(24))
    flow = Flow(Paragraph("First"), child, gap=pt(14))
    assert flow.children[1] is child
    assert "row-gap: 14pt;" in _styles(Document(flow))
    assert "margin-top: 24pt;" in _styles(Document(flow))
    assert 'data-fb-flow="true"' in Document(flow).to_html()
    assert "display: flex" not in _styles(Document(flow))
    with pytest.raises(AuthoringError, match="wrap text"):
        Flow("bare text")


@pytest.mark.parametrize("position", [0, 1, 2])
def test_flow_accepts_fragment_components_in_every_sibling_position(position):
    @component
    def Notes():
        return Fragment(Paragraph("Note A"), Fragment(Paragraph("Note B")))

    siblings = [Paragraph("First"), Paragraph("Last")]
    siblings.insert(position, Notes())
    plan = Document(Flow(Fragment(), siblings, Fragment())).compile()
    assert plan.html.count("<div") == 1
    assert plan.html.count("<p ") == 4
    assert sum(source.path == ("Notes", "p") for source in plan.sources) == 2
    expected = ["First", "Last"]
    expected[position:position] = ["Note A", "Note B"]
    assert [plan.html.index(value) for value in expected] == sorted(plan.html.index(value) for value in expected)


def test_flow_ignores_empty_component_content_and_checks_text_inside_fragments():
    @component
    def Empty():
        return Fragment()

    @component
    def BareText():
        return Fragment("Unwrapped")

    plan = Document(Flow(Empty(), Paragraph("Only"), Empty())).compile()
    assert plan.html.count("<p ") == 1
    for siblings in ((BareText(), Paragraph("After")), (Paragraph("Before"), BareText())):
        with pytest.raises(AuthoringError, match="wrap text") as caught:
            Flow(*siblings)
        assert caught.value.path == ("BareText",)


@pytest.mark.parametrize("name", ["data-fb-flow", "DATA-FB-FLOW"])
def test_flow_rejects_conflicting_layout_annotations(name):
    with pytest.raises(AuthoringError, match="owns"):
        Flow(Paragraph("Text"), attrs={name: "false"})


def test_renderer_applies_metadata_per_document_without_shared_engine_mutation(monkeypatch):
    import fullbleed
    calls = []
    class Engine:
        def __init__(self, **options):
            calls.append(options)
        def compile_pdf(self, html, css):
            self.html = html
            return self
        def export_render_time_pagination_trace(self):
            return {"events": []}
        def render_pdf(self):
            return self.html.encode()
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
        def compile_pdf(self, html, css):
            self.html = html
            return self
        def export_render_time_pagination_trace(self):
            return {"events": [{"reason": "forced_unsplittable_full_frame", "page": 2}]}
        def render_pdf(self):
            pytest.fail("An overflowing document must not be returned")
    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    with pytest.raises(AuthoringError) as caught:
        Document(Paragraph("Huge"), title="Invoice").render_pdf(Renderer())
    assert caught.value.code == "layout.unsplittable"
    assert caught.value.path == ("Invoice", "page 2")
    assert isinstance(caught.value, LayoutError)
    assert caught.value.sources == ()


@pytest.mark.parametrize("identifiers", [("n1", "n1", "unknown"), ("n1", "n3")])
def test_native_fragment_sources_resolve_without_guessing_an_owner(monkeypatch, identifiers):
    import fullbleed
    @component
    def Notice(text: str):
        return Box(Paragraph(text))
    document = Document(Notice("First"), Notice("Second"), title="Report")
    class Engine:
        def __init__(self, **options):
            pass
        def compile_pdf(self, html, css):
            self.html = html
            return self
        def export_render_time_pagination_trace(self):
            return {"events": [{"reason": "forced_unsplittable_full_frame", "page": 2,
                                "source_ids": identifiers}]}
        def render_image_pages(self, *args, **kwargs):
            pytest.fail("An overflowing document must not be returned")
    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    with pytest.raises(LayoutError) as caught:
        document.render_image_pages(Renderer())
    error = caught.value
    expected_ids = ("n1",) if identifiers[1] == "n1" else ("n1", "n3")
    assert tuple(source.node_id for source in error.sources) == expected_ids
    assert all(source.path == ("Notice", "div") and source.file == __file__ for source in error.sources)
    assert error.path == (("Report", "page 2", "Notice", "div") if len(error.sources) == 1 else ("Report", "page 2"))
    assert error.at("Batch").sources == error.sources
    assert error.at("Batch").page == 2
    assert error.to_dict()["sources"][0]["path"] == ["Notice", "div"]


def test_export_links_actual_relative_stylesheet(tmp_path):
    document = Document(Paragraph("A"))
    document.emit_artifacts(html_path=tmp_path / "html/report.html", css_path=tmp_path / "styles/report.css")
    assert 'href="../styles/report.css"' in (tmp_path / "html/report.html").read_text()
    assert (tmp_path / "styles/report.css").read_text() == document.to_css()


@pytest.mark.parametrize("selection", ["one", "multiple", "unknown"])
def test_native_split_rejection_retains_scope_and_resolves_exact_cell_sources(monkeypatch, selection):
    import fullbleed
    document = Document(DataTable(["A"], columns=[Column("Item", str), Column("Amount", str)]), title="Invoice")
    plan = document.compile()
    cells = [source for source in plan.sources if source.path[-1] == "cell"]
    ids = {"one": [cells[0].node_id] * 2, "multiple": [source.node_id for source in cells], "unknown": ["unknown"]}[selection]

    class Engine:
        def __init__(self, **options):
            pass
        def compile_pdf(self, html, css):
            self.html = html
            return self
        def export_render_time_pagination_trace(self):
            return {"events": [{"reason": "forced_unsplittable_full_frame", "page": 2, "source_ids": ["n1"],
                                "split_failure": {"reason": "table_cell_alignment", "source_ids": ids}}]}
        def render_pdf(self, *args):
            pytest.fail("A failed split must not return a PDF")

    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    with pytest.raises(LayoutError) as caught:
        document.render_pdf(Renderer())
    error = caught.value
    assert error.reason == "table_cell_alignment"
    assert error.sources == (plan.sources[0],)
    assert error.failure_sources == (tuple(cells[:1]) if selection == "one" else tuple(cells) if selection == "multiple" else ())
    location = cells[0].path if selection == "one" else plan.sources[0].path if selection == "unknown" else ()
    assert error.path == ("Invoice", "page 2", *location)
    assert "top alignment" in error.message
    assert error.at("Batch").failure_sources == error.failure_sources
    assert error.at("Batch").reason == error.reason
    assert error.to_dict()["failure_sources"] == [{"node_id": source.node_id, "path": list(source.path),
                                                  "file": source.file, "line": source.line} for source in error.failure_sources]


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
    valid = _styles(Document(Panel(), theme=Theme({"spacing": mm(2)})))
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
    assert "color: red;" in _styles(Document(StyledField("value").with_parts(label=style(color="red"))))


def test_migration_carries_styles_and_keeps_legacy_document_ownership():
    from fullbleed.ui import Document as LegacyDocument, Paragraph as LegacyParagraph, StyleSheet, to_html
    @component(parts=("label",))
    def Notice(message):
        return Paragraph(message, part="label", class_name="custom", style=style(color=token("accent")))
    notice = Notice("Paid <in full>").with_parts(label=style(font_weight=700))
    theme = Theme({"accent": "#315d9b"}, text=style(font_size=pt(99)))
    @LegacyDocument(title="Existing report", styles=StyleSheet({".old": {"color": "gray"}}))
    def Report():
        return [LegacyParagraph("Existing content", class_name="old"), to_legacy(notice, theme=theme)]
    artifact = Report()
    html = to_html(artifact)
    assert 'class="custom" style="margin: 0; color: #315d9b; font-weight: 700;"' in html
    assert "Paid &lt;in full&gt;" in html and "Existing content" in html
    assert "99pt" not in html  # Theme.text remains a document setting.
    assert artifact.title == "Existing report" and ".old { color: gray; }" in artifact.to_css()
    assert "data-fb-id=" not in html and "fbx-" not in html


def test_migration_preserves_fragments_attributes_and_independent_snapshots():
    from fullbleed.ui import Element, render_node
    from fullbleed.ui.experimental import Image
    text = Paragraph('A & "B"', attrs={"aria_hidden": False, "title": '<sample>'})
    node = Fragment("Prefix < ", text, text, Image(src="mark.png", alt="Mark"))
    before = Document(node).compile()
    children = to_legacy(node)
    assert len(children) == 4 and children[0] == "Prefix < "
    first, second, image = children[1:]
    assert all(isinstance(child, Element) for child in (first, second, image))
    assert first is not second and image.tag == "img" and image.children == []
    html = render_node(children)
    assert html.startswith('Prefix &lt; <p aria-hidden="false" title="&lt;sample&gt;"')
    assert html.count('A &amp; &quot;B&quot;') == 2
    first.props["class"] = "changed"
    first.children.append("Changed")
    assert "class" not in second.props and len(second.children) == 1
    assert Document(node).compile() == before


def test_migration_matches_style_order_and_reports_the_same_authoring_path():
    @component
    def Panel():
        return Box(style=style(padding=pt(8), padding_left=pt(12), color=token("accent")))
    panel = Panel().with_style(padding=pt(4))
    with pytest.raises(AuthoringError) as compiled:
        Document(panel).compile()
    with pytest.raises(AuthoringError) as adapted:
        to_legacy(panel)
    assert adapted.value.to_dict() == compiled.value.to_dict()
    theme = Theme({"accent": "blue"})
    assert to_legacy(panel, theme=theme)[0].props["style"] == "padding-left: 12pt; color: blue; padding: 4pt;"


def test_migration_requires_an_explicit_node_boundary():
    from fullbleed.ui import el
    node = Paragraph("New component")
    with pytest.raises(TypeError, match="to_legacy"):
        el("div", node)
    for invalid in ("<p>HTML</p>", el("p", "Existing"), Document(node)):
        with pytest.raises(AuthoringError) as caught:
            to_legacy(invalid)
        assert caught.value.code == "migration.node"
    with pytest.raises(AuthoringError) as caught:
        to_legacy(node, theme={"accent": "blue"})
    assert caught.value.code == "migration.theme"


def test_migration_constructs_deep_trees_without_recursive_conversion():
    node = Paragraph("Deep content")
    for _ in range(1500):
        node = Box(node)
    converted = to_legacy(node)[0]
    for _ in range(1500):
        assert converted.tag == "div"
        converted = converted.children[0]
    assert converted.tag == "p" and converted.children == ["Deep content"]
