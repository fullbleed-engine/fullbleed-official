# SPDX-License-Identifier: MIT
"""Check experimental Flow spacing and pagination in independently read PDFs."""
from __future__ import annotations

import argparse
from contextlib import closing
from dataclasses import asdict
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui.experimental import (
    Box, Document, Flow, Fragment, Heading, Length, Page, PageBreak, Paragraph, Renderer, Row,
    Theme, component, pct, pt, style,
)
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


def close(actual, expected, description):
    assert abs(actual - expected) < .1, (description, actual, expected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    renderer = Renderer(font_files=[str(font)], document_timestamp="2026-10-15T00:00:00Z")
    theme = Theme(text=style(font_family="Inter", font_size=pt(12), line_height=1))
    page = Page((pt(240), pt(240)), margin=pt(20))
    results = []

    def check(name, node, markers, *, paper=page, pages=None):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        document = Document(node, title="Component Flow fixture", page=paper, theme=theme)
        plan = document.compile()
        pdf = document.render_pdf(renderer)
        repeated, glyphs = renderer.engine_for(document).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not glyphs, (name, glyphs)
        assert pdf == repeated, (name, "repeat bytes")
        (folder / "document.pdf").write_bytes(pdf)
        document.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        (folder / "sources.json").write_text(json.dumps([asdict(source) for source in plan.sources], indent=2) + "\n", encoding="utf-8")
        reader = PdfReader(BytesIO(pdf))
        if pages is not None:
            assert len(reader.pages) == pages, (name, len(reader.pages), pages)
        texts = [item.extract_text() for item in reader.pages]
        joined = "".join("".join(text.split()) for text in texts)
        for marker in markers:
            assert joined.count(marker) == 1, (name, "text occurrences", marker, joined.count(marker))
        positions = {marker: [] for marker in markers}
        characters = 0
        independent = pdfium.PdfDocument(pdf)
        try:
            for index in range(len(independent)):
                with closing(independent[index]) as sheet:
                    width, height = sheet.get_size()
                    with closing(sheet.get_textpage()) as text_page:
                        for marker in markers:
                            with closing(text_page.search(marker, match_case=True)) as search:
                                found = search.get_next()
                                if found is not None:
                                    left, _, _, top = text_page.get_charbox(found[0])
                                    positions[marker].append({"page": index + 1, "x": left, "y": height - top})
                                    assert search.get_next() is None, (name, "duplicate marker", marker)
                        for char_index in range(text_page.count_chars()):
                            if not text_page.get_text_range(char_index, 1).strip():
                                continue
                            left, bottom, right, top = text_page.get_charbox(char_index)
                            assert -1 <= left <= right <= width + 1 and -1 <= bottom <= top <= height + 1, (
                                name, "character outside page", index, char_index, (left, bottom, right, top))
                            characters += 1
                    with closing(sheet.render(scale=1.5)) as bitmap:
                        bitmap.to_pil().save(folder / f"independent-page-{index + 1}.png")
        finally:
            independent.close()
        assert all(len(found) == 1 for found in positions.values()), (name, positions)
        points = {marker: found[0] for marker, found in positions.items()}
        results.append({"name": name, "pages": len(reader.pages), "pdf_sha256": sha256(pdf).hexdigest(),
                        "markers": points, "checked_characters": characters, "missing_glyphs": 0,
                        "repeat_bytes_match": True})
        return points, pdf, plan

    def box(marker, **declarations):
        # Integral CSS-pixel box sizes keep existing box-edge snapping out of
        # these tests of parent spacing (one CSS pixel is 0.75 pt).
        return Box(Paragraph(marker), style=style(height=pt(18), **declarations))

    control, _, _ = check("margin-control", Flow(box("ITEM_A"), box("ITEM_B"), gap=pt(12)),
                          ["ITEM_A", "ITEM_B"], pages=1)
    close(control["ITEM_B"]["y"] - control["ITEM_A"]["y"], 30, "gap without margins")
    first = box("ITEM_A", margin_top=pt(6), margin_bottom=pt(9), background="#dce8f5")
    second = box("ITEM_B", margin_top=pt(24), margin_bottom=pt(12), background="#e7eee4")
    spaced = Flow(first, second, gap=pt(12))
    assert spaced.children == (first, second) and spaced.children[1] is second
    positions, _, _ = check("additive-margins", spaced, ["ITEM_A", "ITEM_B"], pages=1)
    close(positions["ITEM_A"]["y"] - control["ITEM_A"]["y"], 6, "first child keeps its own top margin")
    close(positions["ITEM_B"]["y"] - positions["ITEM_A"]["y"], 63, "height + bottom margin + gap + top margin")
    positions, _, _ = check("reused-child", Flow(first, second, gap=pt(3)), ["ITEM_A", "ITEM_B"], pages=1)
    close(positions["ITEM_B"]["y"] - positions["ITEM_A"]["y"], 54, "another parent does not mutate a shared child")

    @component
    def Pair():
        return Fragment(Paragraph("ITEM_A"), Fragment(Paragraph("ITEM_B")))

    @component
    def Empty():
        return Fragment()

    for insertion in range(3):
        siblings = [Paragraph("ITEM_PREV"), Paragraph("ITEM_NEXT")]
        siblings.insert(insertion, Pair())
        order = ["ITEM_PREV", "ITEM_NEXT"]
        order[insertion:insertion] = ["ITEM_A", "ITEM_B"]
        points, _, plan = check(f"fragment-position-{insertion}", Flow(siblings, gap=pt(9)), order, pages=1)
        assert plan.html.count("<div") == 1 and plan.html.count("<p ") == 4
        assert sum(source.path == ("Pair", "p") for source in plan.sources) == 2
        for before, after in zip(order, order[1:]):
            close(points[after]["y"] - points[before]["y"], 21, "fragment contributes consecutive siblings")

    _, simple_pdf, _ = check("empty-control", Flow(Paragraph("ITEM_A"), Paragraph("ITEM_B")),
                             ["ITEM_A", "ITEM_B"], pages=1)
    _, empty_pdf, _ = check("empty-fragments", Flow(Empty(), Fragment(), Paragraph("ITEM_A"),
                                                    Empty(), Paragraph("ITEM_B"), Empty()),
                            ["ITEM_A", "ITEM_B"], pages=1)
    assert empty_pdf == simple_pdf, "Empty boundaries have no layout or PDF effect"

    small_page = Page((pt(200), pt(200)), margin=pt(20))
    markers = [f"ITEM_{index:02d}" for index in range(8)]
    points, _, _ = check("page-boundaries", Flow([
        Box(Paragraph(marker), style=style(height=pt(48), break_inside="avoid", background="#dce8f5"))
        for marker in markers
    ], gap=pt(12)), markers, paper=small_page, pages=4)
    for index, marker in enumerate(markers):
        assert points[marker]["page"] == index // 2 + 1, (marker, points)
        close(points[marker]["y"], control["ITEM_A"]["y"] + 60 * (index % 2), "gap stays between page-local items")

    points, _, _ = check("page-margins", Flow([
        Box(Paragraph(marker), style=style(height=pt(48), margin_top=pt(3), margin_bottom=pt(6), break_inside="avoid"))
        for marker in markers[:4]
    ], gap=pt(12)), markers[:4], paper=small_page, pages=2)
    for index, marker in enumerate(markers[:4]):
        assert points[marker]["page"] == index // 2 + 1, (marker, points)
        close(points[marker]["y"], control["ITEM_A"]["y"] + 3 + 69 * (index % 2), "authored margins remain at page boundaries")

    points, _, _ = check("page-tall-gap", Flow(Paragraph("ITEM_A"), Paragraph("ITEM_B"), gap=pt(300)),
                         ["ITEM_A", "ITEM_B"], paper=small_page, pages=2)
    assert [points[key]["page"] for key in ("ITEM_A", "ITEM_B")] == [1, 2]
    close(points["ITEM_A"]["y"], points["ITEM_B"]["y"], "a page-tall gap cannot manufacture an empty page")

    points, _, _ = check("forced-break", Flow(Paragraph("ITEM_A"), PageBreak(), Paragraph("ITEM_B"), gap=pt(18)),
                         ["ITEM_A", "ITEM_B"], paper=small_page, pages=2)
    assert [points[key]["page"] for key in ("ITEM_A", "ITEM_B")] == [1, 2]
    close(points["ITEM_A"]["y"], points["ITEM_B"]["y"], "a break marker has no leading gap")

    points, _, _ = check("kept-heading", Flow(
        Box(Paragraph("ITEM_BEFORE"), style=style(height=pt(114), break_inside="avoid")),
        Heading("ITEM_HEADING", style=style(font_size=pt(12), font_weight=400)),
        Box(Paragraph("ITEM_BODY"), style=style(height=pt(30), break_inside="avoid")), gap=pt(12)),
        ["ITEM_BEFORE", "ITEM_HEADING", "ITEM_BODY"], paper=small_page, pages=2)
    assert [points[key]["page"] for key in ("ITEM_BEFORE", "ITEM_HEADING", "ITEM_BODY")] == [1, 2, 2]
    close(points["ITEM_HEADING"]["y"], control["ITEM_A"]["y"], "kept chain starts without a parent gap")
    close(points["ITEM_BODY"]["y"] - points["ITEM_HEADING"]["y"], 24, "kept chain retains its internal gap")

    points, _, _ = check("nested-flow", Flow(
        Paragraph("ITEM_A"), Flow(Paragraph("ITEM_B"), Paragraph("ITEM_C"), gap=pt(6)),
        Paragraph("ITEM_D"), gap=pt(12)), ["ITEM_A", "ITEM_B", "ITEM_C", "ITEM_D"], pages=1)
    for before, after, distance in (("ITEM_A", "ITEM_B", 24), ("ITEM_B", "ITEM_C", 18), ("ITEM_C", "ITEM_D", 24)):
        close(points[after]["y"] - points[before]["y"], distance, "nested flow owns only its sibling gaps")

    points, _, _ = check("baseline-alignment", Row(
        Flow(Box(style=style(height=pt(18), background="#dce8f5")), Paragraph("ITEM_LEFT"), gap=pt(9)),
        Paragraph("ITEM_RIGHT"), align="baseline"), ["ITEM_LEFT", "ITEM_RIGHT"], pages=1)
    close(points["ITEM_LEFT"]["y"], points["ITEM_RIGHT"]["y"], "baseline includes the preceding item and gap")

    long_markers = [f"ITEM_LONG_{index:03d}" for index in range(30)]
    check("paragraph-continuation", Flow(Paragraph("ITEM_START"), Paragraph(" ".join(long_markers)),
                                         Paragraph("ITEM_END"), gap=pt(12)),
          ["ITEM_START", *long_markers, "ITEM_END"], paper=small_page)
    assert results[-1]["pages"] > 1

    for name, gap, height, expected in (("font-gap", Length(1, "em"), None, 24),
                                       ("percent-auto", pct(10), None, 12),
                                       ("percent-definite", pct(10), pt(120), 24)):
        points, _, _ = check(name, Flow(Paragraph("ITEM_A"), Paragraph("ITEM_B"), gap=gap,
                                       style=style(height=height) if height is not None else None),
                             ["ITEM_A", "ITEM_B"], pages=1)
        close(points["ITEM_B"]["y"] - points["ITEM_A"]["y"], expected, name)

    points, _, _ = check("percent-continuation", Flow([
        Box(Paragraph(marker), style=style(height=pt(36), break_inside="avoid")) for marker in markers[:5]
    ], gap=pct(10), style=style(height=pt(240))), markers[:5], paper=small_page, pages=2)
    for index, marker in enumerate(markers[:5]):
        expected_page, row = (1, index) if index < 3 else (2, index - 3)
        assert points[marker]["page"] == expected_page, (marker, points)
        close(points[marker]["y"], control["ITEM_A"]["y"] + 60 * row, "resolved percentage gap survives fragmentation")

    receipt = {"ok": True, "version": metadata.version("fullbleed"), "runtime": str(fullbleed.__file__),
               "cases": results, "scope": "Experimental native block Flow: additive non-collapsing child margins, transparent and empty fragments, independent parent gaps, page and forced breaks, kept headings, nesting, paragraph continuation and physical/font/percentage gap resolution. Browser layout, conformance and general overflow are not covered."}
    (args.out / "verification.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(results), "pages": sum(item["pages"] for item in results)}))


if __name__ == "__main__":
    main()
