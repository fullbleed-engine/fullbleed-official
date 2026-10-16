from dataclasses import FrozenInstanceError
from hashlib import sha256
from html.parser import HTMLParser

import pytest

from fullbleed.ui.experimental import AuthoringError, Document, Theme, style, token
from fullbleed.ui.experimental import bootstrap as bs


class Markup(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.tags, self.text = [], []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def handle_data(self, data):
        self.text.append(data)


def markup(node):
    return Markup(node.to_html())


def test_exact_bootstrap_500_asset_and_complete_component_family_catalog():
    assert bs.VERSION == "5.0.0"
    assert sha256(bs.stylesheet().encode()).hexdigest() == "04083fcecdd9e19b2acd1e0073437aa07c98d230f0198d7f1fd470ab220fef64"
    # Independently transcribed from the v5.0.0 tag's site/data/sidebar.yml.
    expected = "accordion alerts badge breadcrumb buttons button-group card carousel close-button collapse dropdowns list-group modal navs-tabs navbar offcanvas pagination popovers progress scrollspy spinners toasts tooltips".split()
    assert set(bs.COMPONENT_FAMILIES) == set(expected)
    assert all(callable(getattr(bs, name)) for family in bs.COMPONENT_FAMILIES.values() for name in family)


def test_resources_follow_nested_components_and_are_not_globally_registered():
    card = bs.Card(bs.Paragraph("Saved <safely>"), title="Review & approve", footer=bs.Badge("Ready"))
    doc = Document(card, card)
    embedded = doc.to_html(include_styles=True)
    assert embedded.count("Bootstrap v5.0.0") == 1
    assert "Saved &lt;safely&gt;" in embedded and "Review &amp; approve" in embedded
    assert '<div class="card-body"' in embedded
    assert "Bootstrap" not in Document("Plain").to_css()
    bare = doc.to_html(include_styles=False)
    assert "<style" not in bare and "style=" not in bare and 'class="card"' in bare
    assert doc.compile() == doc.compile()


def test_card_exposes_styles_for_owned_sections_without_touching_nested_cards():
    nested = bs.Card("Nested", title="Nested title")
    outer = bs.Card(nested, title="Outer title").with_parts(title=style(color="#246810"), body=style(padding="2rem"))
    assert outer.parts == ("header", "body", "title", "subtitle", "footer")
    parsed = markup(outer)
    titles = [attrs for tag, attrs in parsed.tags if tag == "h2"]
    assert titles[0]["style"] == "color: #246810;"
    assert "style" not in titles[1]
    assert markup(bs.Card("No footer").with_parts(footer=style(color="blue"))).text == ["No footer"]


def test_literal_classes_checked_utilities_and_local_styles_compose():
    value = bs.classes("card chosen", ["chosen", {"special": True, "omit": False}], None)
    assert value == "card chosen special"
    assert bs.utilities(p=3, mt={"md": 2}, d={"print": "none"}, text=("primary", "center")) == "p-3 mt-md-2 d-print-none text-primary text-center"
    node = bs.Card("Content", class_name=["custom", {"selected": True}], style=style(color=token("ink")))
    html = node.to_html(include_styles=True, theme=Theme({"ink": "#123456"}))
    assert 'class="card custom selected"' in html and "color: #123456;" in html
    assert "style=" not in node.to_html(include_styles=False)
    with pytest.raises(AuthoringError, match="has no class"):
        bs.utilities(m=-1)  # Negative margin utilities are disabled in the shipped 5.0.0 CSS.
    with pytest.raises(AuthoringError, match="breakpoint"):
        bs.utilities(p={"tablet": 2})
    for name in ("clearfix", "text-truncate", "visually-hidden", "position-absolute", "translate-middle",
                 "border-primary", "rounded-pill", "flex-md-wrap", "align-items-lg-center", "gap-5",
                 "order-xxl-last", "offset-xl-11", "row-cols-sm-6", "pe-lg-5", "w-100", "lh-sm",
                 "text-decoration-underline", "overflow-auto", "shadow-lg", "user-select-none", "pe-none"):
        assert bs.utilities(**{name: True}) == name


def test_grid_and_table_structure_do_not_require_html():
    grid = bs.Container(bs.Row(bs.Col("First", span={"xs": 12, "md": 8}), bs.Col("Second", span={"md": 4}), gutter=3))
    html = grid.to_html()
    assert 'class="row g-3"' in html and 'class="col-12 col-md-8"' in html
    assert 'class="col-md-4"' in html
    table = bs.DataTable([{"name": "Ada", "total": 12}], columns=[bs.Column("Name", lambda r: r["name"]),
                         bs.Column("Total", lambda r: r["total"], align="right")], striped=True, small=True)
    tags = markup(table).tags
    assert any(tag == "table" and "table-striped" in attrs["class"] for tag, attrs in tags)
    assert sum(tag == "th" and attrs.get("scope") == "col" for tag, attrs in tags) == 2
    assert "Ada" in table.to_html()


def test_group_ids_and_states_are_authored_without_javascript_controls():
    node = bs.Box(
        bs.Accordion(bs.AccordionItem("First", "Included", key="first"),
                     bs.AccordionItem("Second", "Hidden", open=False), id="questions"),
        bs.Tabs(bs.Tab("Summary", "Selected", key="summary"), bs.Tab("Notes", "Inactive", key="notes"),
                id="record", active="summary"),
        bs.Carousel(bs.CarouselItem("Slide one"), bs.CarouselItem("Slide two"), active=1),
    )
    parsed = markup(node)
    ids = {attrs["id"] for _, attrs in parsed.tags if "id" in attrs}
    references = [attrs["aria-labelledby"] for _, attrs in parsed.tags if "aria-labelledby" in attrs]
    assert set(references) <= ids
    assert "questions-first-heading" in ids and "record-summary-panel" in ids
    assert sum("hidden" in attrs for _, attrs in parsed.tags) == 3
    assert not any(tag == "script" or any(key.startswith(("data-bs-", "on")) for key in attrs) for tag, attrs in parsed.tags)
    assert not any(attrs.get("role") in {"tab", "tablist", "dialog"} for _, attrs in parsed.tags)
    assert node.to_html() == node.to_html()


def test_form_label_relationships_are_generated_once_without_mutation():
    control = bs.Input(type="email", value="demo@example.test", valid=True)
    floating = bs.FloatingLabel(control, "Email", id="email")
    parsed = markup(floating)
    assert ("label", {"for": "email", "data-fb-id": "n3"}) in parsed.tags
    input_attrs = next(attrs for tag, attrs in parsed.tags if tag == "input")
    assert input_attrs["id"] == "email" and input_attrs["placeholder"] == " "
    assert "is-valid" in input_attrs["class"]
    assert "id=" not in control.to_html().replace("data-fb-id=", "source=")
    check = markup(bs.Check("Send a copy", id="copy", checked=True, switch=True))
    assert any(tag == "input" and "checked" in attrs and attrs["id"] == "copy" for tag, attrs in check.tags)
    assert any(tag == "label" and attrs["for"] == "copy" for tag, attrs in check.tags)


def test_progress_range_and_disabled_links_have_explicit_semantics():
    parsed = markup(bs.Progress(bs.ProgressBar(30, max=60, label="Half complete", striped=True)))
    bar = next(attrs for _, attrs in parsed.tags if attrs.get("role") == "progressbar")
    assert "width: 50.0%;" in bar["style"]
    assert bar["aria-valuenow"] == "30" and bar["aria-valuemax"] == "60"
    disabled = markup(bs.Button("Unavailable", href="/purchase", disabled=True))
    assert disabled.tags[0][1].get("href") is None
    assert disabled.tags[0][1]["aria-disabled"] == "true"
    printed_link = Markup(Document(bs.Button("View", href="https://example.test/order")).compile(target="pdf").html)
    assert sum(tag == "a" and attrs.get("href") == "https://example.test/order" for tag, attrs in printed_link.tags) == 1
    assert any("z-index: auto" in attrs.get("style", "") for _, attrs in printed_link.tags)


@pytest.mark.parametrize("build, code", [
    (lambda: bs.Col(span=13), "bootstrap.column"),
    (lambda: bs.Col(span=True), "bootstrap.column"),
    (lambda: bs.Button("X", outline=True, variant="link"), "bootstrap.option"),
    (lambda: bs.Card("X", body=False, title="No body"), "bootstrap.card"),
    (lambda: bs.Tabs(bs.Tab("A", key="same"), bs.Tab("B", key="same"), id="tabs"), "bootstrap.items"),
    (lambda: bs.Tabs(bs.Tab("A"), id="tabs", active=True), "bootstrap.active"),
    (lambda: bs.Tabs(bs.Tab("A"), id="tabs", active="missing"), "bootstrap.active"),
    (lambda: bs.Accordion(id="empty"), "bootstrap.items"),
    (lambda: bs.Collapse("X", open="false"), "bootstrap.option"),
    (lambda: bs.ProgressBar(float("nan")), "bootstrap.progress"),
    (lambda: bs.ProgressBar(101), "bootstrap.progress"),
    (lambda: bs.Range(step=0), "bootstrap.range"),
    (lambda: bs.Range(min=1, max=1), "bootstrap.range"),
    (lambda: bs.FloatingLabel(bs.Paragraph("X"), "Label", id="x"), "bootstrap.floating"),
    (lambda: bs.Check("X", id="has space"), "bootstrap.id"),
    (lambda: bs.Button("X", attrs={"type": "submit"}), "bootstrap.attribute"),
])
def test_invalid_inputs_fail_at_the_component_boundary(build, code):
    with pytest.raises(AuthoringError) as caught:
        build()
    assert caught.value.code == code
    assert caught.value.path


def test_item_descriptors_snapshot_children_and_reject_mutation():
    children = ["Original"]
    tab = bs.Tab("Title", children)
    children.append("Later")
    assert tab.children == ("Original",)
    with pytest.raises(FrozenInstanceError):
        tab.key = "changed"
    cyclic = []
    cyclic.append(cyclic)
    with pytest.raises(AuthoringError, match="Cyclic"):
        bs.Tabs(cyclic, id="tabs")


def test_static_overlays_and_spinner_resource_is_scoped_and_no_javascript_api():
    html = Document(bs.Modal("Body", title="Details", id="details"),
                    bs.Offcanvas("Supplement", title="Notes", id="notes"), bs.Spinner()).to_html(include_styles=True)
    assert "fb-bs-modal" in html and "fb-bs-offcanvas" in html and "animation: none" in html
    assert "bootstrap.bundle" not in html and "<script" not in html
    with pytest.raises(TypeError):
        Document(bs.Card("X")).to_html(include_scripts=True)


def test_print_form_values_selection_and_followup_styles_survive_without_duplicate_controls():
    control = bs.Input(value="Preserved value", id="input", class_name="custom").with_style(color="#123456")
    select = bs.Select(bs.Option("Unselected"), bs.Option("Chosen", attrs={"selected": True}))
    multi = bs.Select(bs.Option("One", attrs={"selected": True}), bs.Option("Two"),
                      bs.Option("Three", attrs={"selected": True}), multiple=True)
    doc = Document(control, select, multi, bs.Textarea("Line one\nLine two"), bs.Input(type="password", value="private"))
    browser = doc.compile()
    printed = doc.compile(target="pdf")
    assert '<input' in browser.html and '<select' in browser.html and 'value="Preserved value"' in browser.html
    assert "<input" not in printed.html and "<select" not in printed.html
    assert "Unselected" not in printed.html and "Two" not in printed.html and "private" not in printed.html
    assert "Preserved value" in printed.html and "Chosen" in printed.html and "One, Three" in printed.html
    assert 'id="input"' in printed.html and "custom fb-bs-print-input" in printed.html and "color: #123456" in printed.html
    assert "Line one\nLine two" in printed.html
    assert printed == doc.compile(target="pdf")
    assert browser == doc.compile()  # Print compilation never mutates the HTML tree.


def test_pdf_preparation_uses_the_print_representation(monkeypatch):
    import fullbleed
    from fullbleed.ui.experimental import Renderer
    recorded = []
    class Engine:
        def __init__(self, **options):
            pass
        def compile_pdf(self, html, css):
            recorded.append(html)
            return self
        def export_render_time_pagination_trace(self):
            return {"events": []}
    monkeypatch.setattr(fullbleed, "PdfEngine", Engine)
    Document(bs.Input(value="Visible on paper")).prepare(Renderer())
    assert "Visible on paper</div>" in recorded[0] and "<input" not in recorded[0]


def test_nested_utility_mappings_fail_without_recursion():
    cyclic = {}
    cyclic["md"] = cyclic
    with pytest.raises(AuthoringError, match="nested mappings"):
        bs.utilities(p=cyclic)


def test_real_catalog_compiles_both_targets_and_uses_known_classes():
    from pathlib import Path
    import runpy
    samples = runpy.run_path(str(Path(__file__).resolve().parents[1] / "examples/component_bootstrap/catalog.py"))["samples"]()
    assert set(bs.COMPONENT_FAMILIES) <= samples.keys()
    for name, node in samples.items():
        doc = Document(node)
        for target in ("html", "pdf"):
            source = doc.compile(target=target)
            for _, attrs in Markup(source.html).tags:
                for class_name in attrs.get("class", "").split():
                    # .accordion is the documented structural container; 5.0.0
                    # styles its descendants without a rule for that class alone.
                    assert class_name == "accordion" or class_name in bs.class_names() or class_name.startswith(("fb-bs-", "fbx-")), (name, class_name)
            assert "<script" not in source.html


def test_contextual_and_responsive_options_exist_in_pinned_bootstrap():
    nodes = []
    for color in ("primary", "secondary", "success", "danger", "warning", "info", "light", "dark"):
        nodes.extend((bs.Alert("Text", variant=color), bs.Badge("Text", variant=color),
                      bs.Button("Text", variant=color, outline=True), bs.Card("Text", variant=color, outline=True),
                      bs.Table(variant=color), bs.ListGroupItem("Text", variant=color),
                      bs.Link("Text", href="#x", color=color), bs.Spinner(variant=color)))
    for breakpoint in ("sm", "md", "lg", "xl", "xxl"):
        nodes.extend((bs.Container(fluid=breakpoint), bs.Col(span={breakpoint: 12}, offset={breakpoint: 1}),
                      bs.Row(columns={breakpoint: 2}, gutter={breakpoint: 3}), bs.Table(responsive=breakpoint),
                      bs.ListGroup(horizontal=breakpoint), bs.Navbar(expand=breakpoint)))
    nodes.extend((bs.ListGroup(bs.ListGroupItem("Numbered"), numbered=True), bs.Ratio("Wide", ratio="21x9")))
    for node in nodes:
        for _, attrs in markup(node).tags:
            assert set(attrs.get("class", "").split()) <= bs.class_names() | {"fb-bs-spinner"}


def test_numbered_list_printing_snapshots_each_ordinal_without_changing_html_items():
    item = bs.ListGroupItem("One")
    document = Document(bs.ListGroup(item, bs.ListGroupItem("Two"), numbered=True))
    printed = document.compile(target="pdf").html
    assert "1. One" in printed and "2. Two" in printed
    assert "1. One" not in document.to_html() and "2. Two" not in document.to_html()
    assert "1. One" not in Document(bs.ListGroup(item)).compile(target="pdf").html
