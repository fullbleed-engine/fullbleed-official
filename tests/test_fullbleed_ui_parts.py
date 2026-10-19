"""Public customization targets must be real, explicit and private by default."""
from dataclasses import FrozenInstanceError
from html.parser import HTMLParser

import pytest

from fullbleed.ui.experimental import (
    AuthoringError, Box, Column, DataTable, Document, Fragment, Node, Paragraph,
    Theme, component, pt, style, styled, token,
)
from fullbleed.ui.experimental import bootstrap as bs


class Elements(HTMLParser):
    def __init__(self, node, **options):
        super().__init__()
        self.tags = []
        self.feed(Document(node, **options).to_html())

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def styles(self, tag="p"):
        return [attrs.get("style", "") for name, attrs in self.tags if name == tag]


@component(parts=("label",), optional_parts=("note",))
def Field(message="Ready", note=None):
    return Box(Paragraph(message, part="label", style=style(color="blue")),
               Paragraph(note, part="note") if note is not None else None)


def test_required_part_cannot_silently_bind_to_a_private_child():
    @component(parts=("label",))
    def Summary():
        return Box(Field())

    with pytest.raises(AuthoringError) as caught:
        Summary()
    assert caught.value.code == "component.part_missing"
    assert caught.value.path == ("Summary", "label")
    assert "forward_parts()" in caught.value.message
    assert "optional" in caught.value.message


def test_required_and_optional_names_are_distinct_captured_contracts():
    required, optional = ["label"], ["note"]

    @component(parts=iter(required), optional_parts=iter(optional))
    def Notice(note=None):
        return Box(Paragraph("Notice", part="label"), Paragraph(note, part="note") if note else None)

    required.clear()
    optional.clear()
    assert Notice.parts == ("label", "note") and Notice.optional_parts == ("note",)
    assert Notice().parts == Notice("Present").parts == Notice.parts
    absent = Notice()
    assert absent.with_parts(note=style(color=token("unused"))) is absent
    assert absent.required_tokens == ()
    assert "color: red;" in Elements(Notice("Present").with_parts(note=style(color="red"))).styles()[1]
    with pytest.raises(FrozenInstanceError):
        absent.optional_parts = ()


def test_forwarding_survives_adding_an_internal_container_without_extra_markup():
    calls = []

    @component(parts=("heading",))
    def Summary(wrap):
        calls.append(wrap)
        child = Field().forward_parts(heading="label")
        return Box(child) if wrap else child

    for wrap in (False, True):
        original = Summary(wrap)
        updated = original.with_parts(heading=style(color="red"))
        assert original.parts == updated.parts == ("heading",)
        assert Elements(original).styles() == ["color: blue;"]
        assert Elements(updated).styles() == ["color: red;"]
        # Forwarding is authoring metadata, not an extra element or CSS selector.
        assert [tag for tag, _ in Elements(original).tags] == [tag for tag, _ in Elements(updated).tags]
    assert calls == [False, True]
    child = Field()
    assert Document(child).compile() == Document(child.forward_parts(heading="label")).compile()


def test_only_selected_child_parts_are_forwarded_and_private_names_stay_private():
    @component(parts=("label",))
    def Private():
        return Paragraph("Private", part="label", style=style(color="green"))

    @component(parts=("label",))
    def Child():
        return Box(Paragraph("Public", part="label", style=style(color="blue")), Private())

    @component(parts=("heading",))
    def Parent():
        return Box(Child().forward_parts(heading="label"))

    parent = Parent()
    assert Elements(parent.with_parts(heading=style(color="red"))).styles() == ["color: red;", "color: green;"]
    assert Elements(parent).styles() == ["color: blue;", "color: green;"]
    with pytest.raises(AuthoringError, match="Available parts: heading"):
        parent.with_parts(label=style(color="red"))


def test_forwarding_can_cross_multiple_deliberate_component_boundaries():
    @component(parts=("heading",))
    def Middle():
        return Box(Field().forward_parts(heading="label"))

    @component(parts=("title",))
    def Outer():
        return Box(Middle().forward_parts(title="heading"))

    assert Elements(Outer().with_parts(title=style(color="purple"))).styles() == ["color: purple;"]


def test_shared_child_occurrences_do_not_share_overrides():
    shared = Field()
    parent = Box(shared.forward_parts(selected="label"), shared)
    assert parent.parts == ("selected",)
    updated = parent.with_parts(selected=style(color="red"))
    assert Elements(updated).styles() == ["color: red;", "color: blue;"]
    assert Elements(parent).styles() == ["color: blue;", "color: blue;"]
    assert Elements(shared).styles() == ["color: blue;"]


def test_parent_and_child_targets_can_be_overridden_together_in_either_order():
    @component(parts=("panel", "heading"))
    def Panel():
        return Box(Field().forward_parts(heading="label"), part="panel")

    options = {"panel": style(background="white"), "heading": style(color="red")}
    for overrides in (options, dict(reversed(tuple(options.items())))):
        updated = Panel().with_parts(**overrides)
        elements = Elements(updated)
        assert elements.styles("div")[0] == "background: white;"
        assert elements.styles() == ["color: red;"]


def test_owned_and_forwarded_targets_with_the_same_public_name_are_all_updated():
    @component(parts=("label",))
    def Group():
        return Box(Paragraph("Local", part="label"), Field().forward_parts("label"), Field())

    assert Elements(Group().with_parts(label=style(color="red"))).styles() == ["color: red;", "color: red;", "color: blue;"]


def test_forwarded_repeated_table_targets_keep_order_and_empty_states():
    calls = []

    @component(parts=("headings",), optional_parts=("values",))
    def Ledger(rows):
        table = DataTable(rows, columns=[Column("Item", lambda row: calls.append(row) or row)])
        return Box(table.forward_parts(headings="header", values="cell"))

    empty = Ledger([])
    assert empty.with_parts(values=style(color=token("unused"))) is empty
    full = Ledger(["First", "Second"]).with_parts(values=style(color="red"), headings=style(color="green"))
    assert Elements(full).styles("td") == ["color: red;", "color: red;"]
    assert Elements(full).styles("th") == ["color: green;"]
    assert calls == ["First", "Second"]
    assert empty.parts == full.parts == ("headings", "values")


def test_a_required_parent_part_cannot_forward_an_absent_optional_target():
    @component(parts=("details",))
    def Summary(note=None):
        return Box(Field(note=note).forward_parts(details="note"))

    with pytest.raises(AuthoringError) as caught:
        Summary()
    assert caught.value.code == "component.part_missing" and caught.value.path == ("Summary", "details")
    assert Elements(Summary("Present").with_parts(details=style(color="red"))).styles()[1] == "color: red;"


def test_forwarding_is_an_explicit_selection_even_for_a_direct_return():
    @component
    def Summary():
        return Field(note="Private to the parent").forward_parts(heading="label")

    assert Summary().parts == ("heading",)
    with pytest.raises(AuthoringError):
        Summary().with_parts(note=style(color="red"))
    original = Field(note="Note")
    assert original.forward_parts(heading="label").parts == original.parts


def test_a_second_forwarding_call_replaces_the_selection_without_mutating_the_first():
    original = Field(note="Note")
    first = original.forward_parts(heading="label")
    second = first.forward_parts(details="note")
    assert Box(first).parts == ("heading",)
    assert Box(second).parts == ("details",)
    assert Box(original).parts == ()


def test_aliases_to_the_same_target_compose_in_override_call_order():
    parent = Box(Field().forward_parts(first="label", second="label"))
    updated = parent.with_parts(first=style(padding_left=pt(3), color="red"), second=style(padding=pt(12), color="green"))
    declarations = Elements(updated).styles()[0]
    assert declarations.index("padding-left: 3pt;") < declarations.index("padding: 12pt;")
    assert "color: green;" in declarations and "color: red;" not in declarations
    reversed_order = parent.with_parts(second=style(color="green"), first=style(color="red"))
    assert Elements(reversed_order).styles() == ["color: red;"]


def test_forwarded_tokens_and_unstyled_html_use_the_existing_export_contract():
    parent = Box(Field().forward_parts(heading="label"))
    updated = parent.with_parts(heading=style(color=token("accent")))
    assert parent.required_tokens == () and updated.required_tokens == ("accent",)
    assert "color: red;" in updated.to_html(include_styles=True, theme=Theme({"accent": "red"}))
    assert "style=" not in updated.to_html(include_styles=False)
    with pytest.raises(AuthoringError) as caught:
        Document(updated).compile()
    assert caught.value.code == "theme.missing" and caught.value.path == ("Field", "label")


def test_forwarding_and_overrides_handle_deep_owned_trees_iteratively():
    child = Field().forward_parts(heading="label")
    for _ in range(1500):
        child = Box(child)
    assert "color: red;" in child.with_parts(heading=style(color="red")).to_html()


def test_styled_factories_keep_required_and_optional_contracts():
    Custom = styled(Field, padding=pt(4))
    assert Custom.parts == Field.parts and Custom.optional_parts == Field.optional_parts == ("note",)
    assert Elements(Box(Custom().forward_parts("label")).with_parts(label=style(color="red"))).styles() == ["color: red;"]


def test_bootstrap_cards_forward_owned_sections_in_both_export_targets():
    @component(parts=("heading",), optional_parts=("footnote",))
    def Summary():
        return Box(bs.Card("Contents", title="Summary").forward_parts(heading="title", footnote="footer"))

    updated = Summary().with_parts(heading=style(color="red"), footnote=style(color=token("unused")))
    assert updated.required_tokens == ()
    for target in ("html", "pdf"):
        source = Document(updated).compile(target=target)
        assert 'style="color: red;"' in source.html and source.css.count("Bootstrap v5.0.0") == 1


@pytest.mark.parametrize("action", [
    lambda: Field().forward_parts(),
    lambda: Paragraph("Owned", part="label").forward_parts("label"),
    lambda: Field().forward_parts("missing"),
    lambda: Field().forward_parts(heading="missing"),
    lambda: Field().forward_parts("label", "label"),
    lambda: Field().forward_parts("label", label="note"),
    lambda: Field().forward_parts(heading=1),
    lambda: Field().forward_parts(**{"bad name": "label"}),
])
def test_invalid_forwarding_fails_before_a_document_is_compiled(action):
    with pytest.raises(AuthoringError):
        action()


@pytest.mark.parametrize("options", [
    {"parts": ("label",), "optional_parts": ("label",)},
    {"optional_parts": None}, {"optional_parts": "label"}, {"optional_parts": {"label"}},
    {"optional_parts": ("note", "note")}, {"optional_parts": ("bad name",)},
])
def test_invalid_optional_declarations_fail_at_decoration(options):
    with pytest.raises(AuthoringError):
        component(**options)


def test_forwarded_exports_must_be_declared_when_the_parent_seals_its_contract():
    @component(parts=())
    def Closed():
        return Box(Field().forward_parts("label"))

    with pytest.raises(AuthoringError, match="missing from the component declaration: label"):
        Closed()


def test_an_optional_only_component_can_legitimately_have_no_elements():
    @component(optional_parts=("note",))
    def MaybeNote():
        return Fragment()

    assert MaybeNote.parts == ("note",) and MaybeNote.optional_parts == ("note",)
    node = MaybeNote()
    assert node.with_parts(note=style(color="red")) is node
