from __future__ import annotations

import inspect
import json
from pathlib import Path

import pytest

from fullbleed.ui import Document, Element, Box, component, el, style, to_html
from fullbleed.ui.accessibility import A11yContract, A11yValidationError
from fullbleed.ui.style import StyleWarning

FIXTURE = json.loads((Path(__file__).parent / "fixtures/fullbleed_ui/portable.json").read_text(encoding="utf-8"))


def from_spec(spec):
    if isinstance(spec, list):
        return [from_spec(child) for child in spec]
    if isinstance(spec, dict):
        return el(spec["tag"], *(from_spec(child) for child in spec.get("children", [])), **spec.get("props", {}))
    return spec


@pytest.mark.parametrize("case", FIXTURE["cases"], ids=lambda case: case["name"])
def test_portable_html(case):
    tree = from_spec(case["node"])
    assert to_html(tree) == case["html"]
    assert to_html(tree) == case["html"]


@pytest.mark.parametrize("case", FIXTURE["styles"], ids=lambda case: case["name"])
def test_portable_style(case):
    assert style(*case["input"]).to_css() == case["css"]


def test_generator_children_are_snapshotted_once_and_seen_by_validation():
    calls = []
    def rows():
        for value in ("one", "two"):
            calls.append(value)
            yield [el("p", value, id="duplicate")]
    tree = Box(rows())
    assert calls == ["one", "two"]
    expected = '<div class="ui-box"><p id="duplicate">one</p><p id="duplicate">two</p></div>'
    assert to_html(tree) == to_html(tree) == expected
    assert calls == ["one", "two"]
    with pytest.raises(A11yValidationError):
        to_html(tree, a11y_mode="raise")


def test_top_level_generator_is_validated_without_consuming_it_twice():
    nodes = (el("p", text) for text in ("one", "two"))
    assert to_html(nodes, a11y_mode="raise") == "<p>one</p><p>two</p>"


def test_child_lists_are_copied_and_shared_elements_are_allowed():
    shared = el("p", "Shared")
    children = [[shared]]
    tree = el("div", children, shared)
    children[0].append("later")
    assert to_html(tree) == "<div><p>Shared</p><p>Shared</p></div>"


def test_document_preserves_component_signature_and_metadata():
    @Document(title="Report", bootstrap=False)
    @component
    def Report(customer: str, *, paid: bool = False):
        """Reusable customer report."""
        return [el("h1", customer), paid and el("p", "Paid")]
    assert inspect.signature(Report) == inspect.signature(Report.__wrapped__)
    assert Report.__doc__ == "Reusable customer report."
    assert Report.__fullbleed_component__ is True
    assert "False" not in Report("Alice & Bob").to_html()
    assert "Alice &amp; Bob" in Report("Alice & Bob").to_html()


@pytest.mark.parametrize("child", [{"amount": 12}, {1, 2}, b"bytes", lambda: el("p", "Forgot call")])
def test_ambiguous_children_fail_instead_of_printing_representations(child):
    with pytest.raises(TypeError, match="component child"):
        el("div", child)


@pytest.mark.parametrize("tag", ["div onclick=x", "p><script", "", "<p", "div/", 12])
def test_tag_names_cannot_break_markup(tag):
    with pytest.raises(ValueError, match="tag name"):
        Element(tag)


@pytest.mark.parametrize("name", ['title" onclick="x', "x y", "x=y", "x>y", "x\x00y"])
def test_attribute_names_cannot_break_markup(name):
    with pytest.raises(ValueError, match="attribute name"):
        to_html(el("div", **{name: "value"}))


def test_void_content_is_not_silently_discarded():
    with pytest.raises(ValueError, match="cannot have children"):
        el("img", "lost", src="image.png")


def test_cycles_fail_with_an_authoring_error():
    children = []
    children.append(children)
    with pytest.raises(ValueError, match="Cyclic"):
        el("div", children)
    tree = el("div")
    tree.children.append(tree)
    with pytest.raises(ValueError, match="Cyclic"):
        to_html(tree)
    with pytest.raises(ValueError, match="Cyclic"):
        to_html(tree, a11y_mode="raise")


@pytest.mark.parametrize("fragment", ['color: red; content: "open', 'color: red; --x: (open', 'color: red; /* open'])
def test_incomplete_styles_warn_and_preserve_complete_declarations(fragment):
    with pytest.warns(StyleWarning, match="incomplete"):
        assert style(fragment).to_css() == "color: red;"


def test_merging_style_does_not_mutate_the_source():
    base = style(color="red", padding="2px")
    merged = style(base, color="blue")
    assert base.to_css() == "color: red; padding: 2px;"
    assert merged.to_css() == "padding: 2px; color: blue;"


def test_style_cycles_and_unordered_fragments_fail_clearly():
    cycle = []
    cycle.append(cycle)
    with pytest.raises(ValueError, match="Cyclic"):
        style(cycle)
    with pytest.raises(TypeError, match="ordered"):
        style({"color: red", "color: blue"})
