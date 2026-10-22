"""Public authoring paths must emit real, correctly owned PDF link objects.

These object checks complement the independent reader and profile fixtures;
they do not establish general PDF/UA conformance or screen-reader behavior.
"""
import io
from html.parser import HTMLParser

import fullbleed
import fullbleed_assets
from fullbleed.components import Document, Renderer
from fullbleed.components.bootstrap import Button, Link
from pypdf import PdfReader
from pypdf.generic import ArrayObject, IndirectObject
import pytest


CSS = """@page{size:300pt 300pt;margin:16pt}
body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}
p{margin:0 0 12pt}h1,h2{font-weight:normal}"""
HTML = """<h1>Report</h1>
<p><a href="https://example.invalid/café?q=✓">{{name}}</a></p>
<p><a href="#details">Details</a></p><h2 id="details">Order details</h2>
<p><a href="#absent">Missing target</a> <a>Plain anchor</a></p>
<a href="https://example.invalid/hidden" style="display:none">Hidden</a>"""


def font():
    return str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))


def reader(pdf):
    return PdfReader(io.BytesIO(bytes(pdf)), strict=True)


def ref_id(reference):
    if isinstance(reference, IndirectObject):
        return reference.idnum
    return reference.indirect_reference.idnum


def structures(root):
    records = {}

    def visit(reference):
        value = reference.get_object() if isinstance(reference, IndirectObject) else reference
        if isinstance(value, ArrayObject):
            for child in value:
                visit(child)
        elif hasattr(value, "get") and value.get("/Type") == "/StructElem":
            key = ref_id(reference)
            assert key not in records, "A structure element must have exactly one parent"
            records[key] = value
            visit(value.get("/K", ArrayObject()))

    visit(root["/K"])
    return records


@pytest.mark.parametrize("profile", ["none", "tagged", "pdfua1", "pdfua2"])
@pytest.mark.parametrize("mode", ["direct", "compiled", "repeat", "reflow", "compact"])
def test_authored_links_preserve_targets_and_recipient_structure(profile, mode):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Authored links",
        font_files=[font()],
    )

    def render():
        if mode == "direct":
            return engine.render_pdf(HTML.replace("{{name}}", "Ada"), CSS)
        compiled = engine.compile_pdf(
            HTML if mode in {"reflow", "compact"} else HTML.replace("{{name}}", "Ada"), CSS
        )
        if mode == "compiled":
            return compiled.render_pdf()
        if mode == "repeat":
            return compiled.render_pdf_batch(2)
        return compiled.render_pdf_reflow_bindings(
            {"name": ["Ada", "Grace"]},
            compression="compact" if mode == "compact" else "throughput",
        )

    pdf = render()
    assert pdf == render()
    document = reader(pdf)
    count = 1 if mode in {"direct", "compiled"} else 2
    assert len(document.pages) == count
    root = document.trailer["/Root"].get("/StructTreeRoot")
    tags = structures(root.get_object()) if root else {}
    assert bool(tags) == (profile != "none")
    link_tags = {key: tag for key, tag in tags.items() if tag["/S"] == "/Link"}
    if root:
        assert len(link_tags) == count * 2
        nums = root.get_object()["/ParentTree"]["/Nums"]
        parents = {int(nums[i]): nums[i+1] for i in range(0, len(nums), 2)}
    destinations = set()
    for index, page in enumerate(document.pages):
        annotations = [ref.get_object() for ref in page["/Annots"]]
        assert len(annotations) == 2
        external, local = annotations
        expected_name = "Grace" if mode in {"reflow", "compact"} and index == 1 else "Ada"
        assert external["/Contents"] == expected_name
        assert external["/A"]["/S"] == "/URI"
        assert external["/A"]["/URI"] == "https://example.invalid/caf%C3%A9?q=%E2%9C%93"
        assert local["/Contents"] == "Details"
        if profile == "pdfua2":
            action = local["/A"]
            assert action["/S"] == "/GoTo"
            destination = action["/D"]
            semantic = action["/SD"]
            target_id = ref_id(semantic[0])
            assert target_id not in destinations, "A recipient must not link into another record"
            destinations.add(target_id)
            assert tags[target_id]["/S"] == "/H2"
            assert ref_id(tags[target_id].raw_get("/Pg")) == ref_id(page)
            assert semantic[1:] == destination[1:]
        else:
            destination = local["/Dest"]
        assert ref_id(destination[0]) == ref_id(page)
        assert destination[1] == "/XYZ"
        for annotation in annotations:
            assert annotation["/Subtype"] == "/Link"
            assert ref_id(annotation.raw_get("/P")) == ref_id(page)
            assert len(annotation["/QuadPoints"]) >= 8
            if root:
                owner = link_tags[ref_id(parents[int(annotation["/StructParent"])])]
                kids = owner["/K"]
                kids = kids if isinstance(kids, ArrayObject) else [kids]
                objects = [child.get_object() for child in kids]
                references = [obj for obj in objects if hasattr(obj, "get") and obj.get("/Type") == "/OBJR"]
                assert len(references) == 1
                assert ref_id(references[0].raw_get("/Obj")) == ref_id(annotation)


def test_fixed_bindings_use_final_recipient_link_text():
    engine = fullbleed.PdfEngine(font_files=[font()])
    compiled = engine.compile_pdf(HTML, CSS)
    document = reader(compiled.render_pdf_bindings({"name": ["Ada", "Grace"]}))
    assert len(document.pages) == 2
    for page, expected in zip(document.pages, ["Ada", "Grace"]):
        annotations = [ref.get_object() for ref in page["/Annots"]]
        assert annotations[0]["/Contents"] == expected
        assert ref_id(annotations[1]["/Dest"][0]) == ref_id(page)


SVG_IMAGE = """<img alt="Open documents" width="40" height="20"
src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='20'%3E%3Crect width='10' height='10' fill='green'/%3E%3C/svg%3E">"""


@pytest.mark.parametrize("profile", ["none", "pdfua2"])
@pytest.mark.parametrize("mode", ["direct", "compiled", "reflow"])
def test_linked_svg_images_emit_actions_in_public_render_paths(profile, mode):
    url = "https://example.invalid/documents"
    html = '<h1>{{label}}</h1><a href="' + url + '">' + SVG_IMAGE + '</a>'
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Linked SVG",
        font_files=[font()],
    )
    if mode == "direct":
        data = engine.render_pdf(html.replace("{{label}}", "Documents"), CSS)
    elif mode == "compiled":
        data = engine.compile_pdf(html.replace("{{label}}", "Documents"), CSS).render_pdf()
    else:
        data = engine.compile_pdf(html, CSS).render_pdf_reflow_bindings({"label": ["Documents"]})
    pdf = reader(data)
    assert len(pdf.pages) == 1
    annotations = [ref.get_object() for ref in pdf.pages[0]["/Annots"]]
    assert len(annotations) == 1
    link = annotations[0]
    assert link["/A"]["/URI"] == url
    assert link["/Contents"] == "Open documents"
    left, bottom, right, top = map(float, link["/Rect"])
    assert right - left == pytest.approx(30, abs=0.001)
    assert top - bottom == pytest.approx(15, abs=0.001)
    if profile == "pdfua2":
        tags = structures(pdf.trailer["/Root"]["/StructTreeRoot"])
        assert sum(tag["/S"] == "/Link" for tag in tags.values()) == 1
        assert any(tag["/S"] == "/Figure" and tag.get("/Alt") == "Open documents" for tag in tags.values())


@pytest.mark.parametrize("style", ["display:none", "visibility:hidden", "opacity:0", ""])
def test_hidden_and_unlinked_svg_images_have_no_actions(style):
    attributes = ' href="https://example.invalid/hidden"' if style else ""
    html = '<h1>Documents</h1><a' + attributes + ' style="' + style + '">' + SVG_IMAGE + '</a>'
    pdf = reader(fullbleed.PdfEngine(font_files=[font()]).render_pdf(html, CSS))
    assert all(not page.get("/Annots") for page in pdf.pages)


@pytest.mark.parametrize("profile", ["none", "pdfua2"])
@pytest.mark.parametrize("shadow", [
    "40pt 35pt #aaa", "40pt 35pt 4pt #aaa", "40pt 35pt #aaa,-10pt -5pt 3pt #777",
])
def test_text_shadows_leave_link_labels_and_click_regions_unchanged(profile, shadow):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Linked shadow",
        font_files=[font()],
    )
    html = '<p><a href="https://example.invalid/report">Read</a></p>'
    plain = reader(engine.render_pdf(html, CSS))
    decorated = reader(engine.render_pdf(html, CSS + "a{text-shadow:" + shadow + "}"))
    expected = [ref.get_object() for ref in plain.pages[0]["/Annots"]]
    actual = [ref.get_object() for ref in decorated.pages[0]["/Annots"]]
    assert len(actual) == len(expected) == 1
    assert actual[0]["/Contents"] == "Read"
    assert actual[0]["/Rect"] == expected[0]["/Rect"]
    assert actual[0]["/QuadPoints"] == expected[0]["/QuadPoints"]


def test_clipped_link_padding_has_no_rectangular_activation_in_the_excluded_corner():
    document = reader(fullbleed.PdfEngine().render_pdf(
        '<a href="https://example.invalid/report">Read</a>',
        """@page{size:200pt 200pt;margin:0}body{margin:0}
        a{display:block;width:100pt;height:100pt;padding:10pt;box-sizing:border-box;
        clip-path:polygon(0 0,100% 0,100% 40%,40% 40%,40% 100%,0 100%)}""",
    ))
    boxes = [list(map(float, ref.get_object()["/Rect"])) for ref in document.pages[0]["/Annots"]]

    def hit(x, y):
        return any(left <= x <= right and bottom <= y <= top for left, bottom, right, top in boxes)

    assert hit(5, 195), "The author's padding is part of the link"
    assert not hit(80, 120), "The concave clip must leave its excluded corner inactive"
    assert not hit(110, 190)


def test_wrapped_link_has_one_logical_owner_across_pages():
    html = '<p><a href="https://example.invalid/report">' + "Read the report. " * 90 + "</a></p>"
    engine = fullbleed.PdfEngine(pdf_profile="tagged", font_files=[font()])
    document = reader(engine.render_pdf(html, CSS.replace("300pt 300pt", "180pt 180pt")))
    assert len(document.pages) > 2
    tags = structures(document.trailer["/Root"]["/StructTreeRoot"])
    links = [tag for tag in tags.values() if tag["/S"] == "/Link"]
    assert len(links) == 1
    total = sum(len(page.get("/Annots", [])) for page in document.pages)
    assert total > len(document.pages)
    objects = [child.get_object() for child in links[0]["/K"]]
    references = [obj for obj in objects if hasattr(obj, "get") and obj.get("/Type") == "/OBJR"]
    assert len(references) == total


@pytest.mark.parametrize("component", [Link, Button])
def test_component_links_preserve_pdf_actions_and_both_html_exports(component):
    url = "https://example.invalid/documents?format=pdf&source=author"
    document = Document(component("Documents", href=url), title="Authored link")

    class Anchors(HTMLParser):
        def __init__(self, html):
            super().__init__()
            self.links = []
            self.feed(html)

        def handle_starttag(self, tag, attrs):
            if tag == "a":
                self.links.append(dict(attrs).get("href"))

    for include_styles in [False, True]:
        assert Anchors(document.to_html(include_styles=include_styles)).links == [url]
    pdf = reader(document.render_pdf(Renderer(font_files=[font()])))
    actions = [ref.get_object()["/A"] for page in pdf.pages for ref in page.get("/Annots", [])]
    assert actions
    assert all(action["/S"] == "/URI" and action["/URI"] == url for action in actions)
