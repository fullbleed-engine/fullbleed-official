"""Composition must keep authored navigation attached to real output pages.

Tagged inputs exercise the page-destination fallback; composition does not
currently preserve their structure trees or establish a tagged output profile.
"""
import base64

import fullbleed
import fullbleed_assets
from pypdf import PdfReader, PdfWriter
from pypdf.generic import DictionaryObject, RectangleObject
import pytest


CSS = """@page{size:300pt 300pt;margin:16pt}
body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}
p{margin:0 0 12pt}h2{font-size:16pt;font-weight:normal}"""


def destination(annotation):
    if "/Dest" in annotation:
        return annotation["/Dest"]
    action = annotation.get("/A")
    return action.get_object().get("/D") if action is not None else None


def annotations(page):
    return [ref.get_object() for ref in page.get("/Annots", [])]


@pytest.mark.parametrize("profile", ["none", "pdfua2"])
@pytest.mark.parametrize("mode", ["link_only", "none", "carry_widgets"])
def test_composition_keeps_overlay_links_offsets_and_repeated_local_targets(tmp_path, profile, mode):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Composition links",
        font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))],
    )
    template = tmp_path / "template.pdf"
    overlay = tmp_path / "overlay.pdf"
    template.write_bytes(bytes(engine.render_pdf(
        '<p><a href="https://example.invalid/template">Template</a></p>'
        '<p><a href="#target">Template details</a></p><h2 id="target">Template target</h2>', CSS)))
    overlay.write_bytes(bytes(engine.render_pdf(
        '<p><a href="https://example.invalid/overlay">Overlay</a></p>'
        '<p><a href="#target">Overlay details</a></p>'
        '<h2 id="target" style="break-before:page">Overlay target</h2>', CSS)))
    source_template = PdfReader(template, strict=True)
    source_overlay = PdfReader(overlay, strict=True)
    assert len(source_template.pages) == 1 and len(source_overlay.pages) == 2
    assert {a["/Contents"] for a in annotations(source_template.pages[0])} == {"Template", "Template details"}
    assert {a["/Contents"] for a in annotations(source_overlay.pages[0])} == {"Overlay", "Overlay details"}
    offsets = [(12.0,18.0), (-8.0,22.0), (30.0,-4.0), (40.0,70.0)]
    plan = [("template", 0, index % 2, dx, dy) for index, (dx,dy) in enumerate(offsets)]
    out = tmp_path / "out.pdf"
    replay = tmp_path / "replay.pdf"
    for path in (out, replay):
        result = fullbleed.finalize_compose_pdf(
            [("template", str(template))], plan, str(overlay), str(path), annotation_mode=mode)
        assert result["ok"] and result["pages_written"] == 4
    assert out.read_bytes() == replay.read_bytes()
    composed = PdfReader(out, strict=True)
    page_ids = [page.indirect_reference.idnum for page in composed.pages]
    for index, page in enumerate(composed.pages):
        expected = []
        if mode != "none":
            expected.extend((a, False) for a in annotations(source_template.pages[0]))
        expected.extend((a, True) for a in annotations(source_overlay.pages[index % 2]))
        actual = annotations(page)
        assert len(actual) == len(expected)
        for after, (before, is_overlay) in zip(actual, expected):
            assert after["/Contents"] == before["/Contents"]
            assert after.raw_get("/P").idnum == page_ids[index]
            assert "/StructParent" not in after
            dx, dy = offsets[index] if is_overlay else (0.0, 0.0)
            translated_rect = [float(v) + (dx if i % 2 == 0 else dy) for i, v in enumerate(before["/Rect"])]
            visible_rect = [max(0.0,translated_rect[0]),max(0.0,translated_rect[1]),
                            min(300.0,translated_rect[2]),min(300.0,translated_rect[3])]
            assert list(map(float, after["/Rect"])) == pytest.approx(visible_rect, abs=1e-4)
            points = list(map(float,after["/QuadPoints"]))
            assert points and len(points)%8 == 0
            if visible_rect == translated_rect:
                expected_points = [float(v) + (dx if i % 2 == 0 else dy) for i,v in enumerate(before["/QuadPoints"])]
                assert points == pytest.approx(expected_points, abs=1e-4)
            else:
                assert [min(points[::2]),min(points[1::2]),max(points[::2]),max(points[1::2])] == pytest.approx(visible_rect, abs=1e-4)
            before_destination = destination(before)
            if before_destination is None:
                assert after["/A"]["/S"] == "/URI"
                assert after["/A"]["/URI"] == before["/A"]["/URI"]
            else:
                after_destination = destination(after)
                target = index // 2 * 2 + 1 if is_overlay else index
                tx, ty = offsets[target] if is_overlay else (0.0, 0.0)
                assert after_destination[0].idnum == page_ids[target]
                assert after_destination[1] == "/XYZ"
                assert float(after_destination[2]) == pytest.approx(float(before_destination[2]) + tx, abs=1e-4)
                assert float(after_destination[3]) == pytest.approx(float(before_destination[3]) + ty, abs=1e-4)
                assert "/SD" not in after.get("/A", {})


def test_composition_reports_a_destination_omitted_from_the_plan(tmp_path):
    engine = fullbleed.PdfEngine(font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))])
    template, overlay, out = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf")]
    template.write_bytes(bytes(engine.render_pdf("<p>Template</p>", CSS)))
    overlay.write_bytes(bytes(engine.render_pdf(
        '<a href="#target">Details</a><h2 id="target" style="break-before:page">Target</h2>', CSS)))
    out.write_bytes(b"existing output")
    with pytest.raises(ValueError, match="destination page is not included in the compose plan"):
        fullbleed.finalize_compose_pdf(
            [("template", str(template))], [("template", 0, 0, 0.0, 0.0)], str(overlay), str(out))
    assert out.read_bytes() == b"existing output"


@pytest.mark.parametrize("profile", ["none", "pdfua2"])
def test_stamping_keeps_overlay_navigation_and_existing_template_annotations(tmp_path, profile):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Stamped navigation",
        font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))])
    template, overlay, out = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf")]
    template.write_bytes(bytes(engine.render_pdf(
        '<p><a href="https://example.invalid/template">Template one</a></p>'
        '<p style="break-before:page"><a href="https://example.invalid/template">Template two</a></p>', CSS)))
    overlay.write_bytes(bytes(engine.render_pdf(
        '<p><a href="https://example.invalid/overlay">Overlay</a></p><p><a href="#target">Details</a></p>'
        '<h2 id="target" style="break-before:page">Target</h2>', CSS)))
    source_template, source_overlay = PdfReader(template, strict=True), PdfReader(overlay, strict=True)
    assert len(source_template.pages) == len(source_overlay.pages) == 2
    assert {a["/Contents"] for a in annotations(source_overlay.pages[0])} == {"Overlay", "Details"}
    fullbleed.finalize_stamp_pdf(str(template), str(overlay), str(out),
                               page_map=[(0,0),(1,1),(0,0),(1,1)], dx=12.0, dy=18.0)
    result = PdfReader(out, strict=True)
    assert len(result.pages) == 2
    page_ids = [p.indirect_reference.idnum for p in result.pages]
    for index, page in enumerate(result.pages):
        existing = annotations(source_template.pages[index])
        added = annotations(source_overlay.pages[index])
        actual = annotations(page)
        assert len(actual) == len(existing) + 2 * len(added)
        for after, before in zip(actual[:len(existing)], existing):
            assert after["/A"]["/URI"] == before["/A"]["/URI"]
            assert list(map(float, after["/Rect"])) == pytest.approx(list(map(float,before["/Rect"])), abs=1e-4)
        for after in actual[len(existing):]:
            assert after.raw_get("/P").idnum == page_ids[index]
            assert "/StructParent" not in after
            target = destination(after)
            if target is not None:
                assert target[0].idnum == page_ids[1]
                assert "/SD" not in after.get("/A", {})
            else:
                assert after["/A"]["/URI"] == "https://example.invalid/overlay"


@pytest.mark.parametrize("method", ["compose", "stamp"])
def test_imported_overlay_link_rect_and_quads_are_clipped_to_its_cropbox(tmp_path, method):
    engine = fullbleed.PdfEngine(font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))])
    template, original, overlay, out = [tmp_path / name for name in ("template.pdf", "original.pdf", "overlay.pdf", "out.pdf")]
    template.write_bytes(bytes(engine.render_pdf("<p>Template</p>", CSS)))
    svg = base64.b64encode(b'<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><rect width="80" height="20" fill="blue"/></svg>').decode()
    original.write_bytes(bytes(engine.render_pdf(
        f'<a href="https://example.invalid/cropped"><img alt="Sample" style="width:80pt;height:20pt" src="data:image/svg+xml;base64,{svg}"></a>', CSS)))
    source = PdfReader(original, strict=True)
    assert len(annotations(source.pages[0])) == 1
    left,bottom,right,top = map(float, annotations(source.pages[0])[0]["/Rect"])
    cropped_left = (left+right)/2
    writer = PdfWriter()
    page = writer.add_page(source.pages[0])
    page.cropbox = RectangleObject([cropped_left,0,300,300])
    with overlay.open("wb") as stream:
        writer.write(stream)
    if method == "compose":
        fullbleed.finalize_compose_pdf([("tpl",str(template))],[("tpl",0,0,8.0,10.0)],
                                      str(overlay),str(out),annotation_mode="none")
    else:
        fullbleed.finalize_stamp_pdf(str(template),str(overlay),str(out),dx=8.0,dy=10.0)
    result = PdfReader(out, strict=True)
    links = annotations(result.pages[0])
    assert len(links) == 1 and links[0]["/A"]["/URI"] == "https://example.invalid/cropped"
    rect = list(map(float, links[0]["/Rect"]))
    assert rect == pytest.approx([cropped_left+8,bottom+10,right+8,min(top+10,300)],abs=1e-4)
    points = list(map(float, links[0]["/QuadPoints"]))
    assert points and len(points)%8 == 0
    assert all(rect[0]-1e-4 <= x <= rect[2]+1e-4 and rect[1]-1e-4 <= y <= rect[3]+1e-4
               for x,y in zip(points[::2],points[1::2]))


def test_stamping_preserves_existing_template_link_owners_and_semantic_destinations(tmp_path):
    tagged = fullbleed.PdfEngine(
        pdf_profile="pdfua2", document_lang="en-US", document_title="Retained template semantics",
        font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))])
    template, overlay, out = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf")]
    template.write_bytes(bytes(tagged.render_pdf(
        '<p><a href="#target">Template details</a></p><h2 id="target">Details</h2>', CSS)))
    overlay.write_bytes(bytes(fullbleed.PdfEngine().render_pdf("<p>Overlay</p>", CSS)))
    original = PdfReader(template, strict=True)
    expected_parent_keys = [int(a["/StructParent"]) for a in annotations(original.pages[0])]
    assert expected_parent_keys
    fullbleed.finalize_stamp_pdf(str(template), str(overlay), str(out))
    result = PdfReader(out, strict=True)
    page = result.pages[0]
    owners = result.trailer["/Root"]["/StructTreeRoot"]["/ParentTree"]["/Nums"]
    parents = {int(owners[i]):owners[i+1].get_object() for i in range(0,len(owners),2)}
    assert [int(a["/StructParent"]) for a in annotations(page)] == expected_parent_keys
    for reference in page["/Annots"]:
        annotation = reference.get_object()
        action = annotation["/A"]
        assert action["/D"][0].idnum == page.indirect_reference.idnum
        target = action["/SD"][0].get_object()
        assert target["/S"] == "/H2" and target.raw_get("/Pg").idnum == page.indirect_reference.idnum
        owner = parents[int(annotation["/StructParent"])]
        assert owner["/S"] == "/Link"
        assert any(isinstance(child.get_object(), DictionaryObject)
                   and child.get_object().get("/Type") == "/OBJR"
                   and child.get_object().raw_get("/Obj").idnum == reference.idnum
                   for child in owner["/K"])
