# SPDX-License-Identifier: MIT
"""Verify explicit layout choices against plain-component reference PDFs."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui.experimental import (
    Box, Document, Flow, Grid, Heading, Page, Paragraph, Renderer, Row, Stack,
    Theme, component, pt, style, styled,
)
from PIL import ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-15T00:00:00Z")
    theme = Theme(text=style(font_family="Inter", font_size=pt(12), line_height=1))
    paper = Page((pt(240), pt(300)), margin=pt(20))
    cases = []

    def check(name, actual, reference, markers, *, page=paper):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        document = Document(actual, title="Component layout choices", page=page, theme=theme)
        expected = Document(reference, title=document.title, page=page, theme=theme)
        plan = document.compile(target="pdf")
        pdf, glyphs = renderer.engine_for(document).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not glyphs, (name, glyphs)
        assert pdf == document.render_pdf(renderer), (name, "repeat bytes")
        reference_pdf = expected.render_pdf(renderer)
        reader = PdfReader(BytesIO(pdf))
        text = "".join("".join(sheet.extract_text().split()) for sheet in reader.pages)
        assert all(text.count(marker) == 1 for marker in markers), (name, text)
        document.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css", target="pdf")
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "reference.pdf").write_bytes(reference_pdf)
        trace = renderer.engine_for(document).export_render_time_pagination_trace(plan.html, plan.css)
        (folder / "pagination-trace.json").write_text(json.dumps(trace, indent=2) + "\n", encoding="utf-8")
        positions = {}
        with closing(pdfium.PdfDocument(pdf)) as rendered, closing(pdfium.PdfDocument(reference_pdf)) as baseline:
            assert len(rendered) == len(baseline), name
            for index in range(len(rendered)):
                with closing(rendered[index]) as sheet, closing(baseline[index]) as expected_sheet:
                    with closing(sheet.render(scale=1.5)) as bitmap, closing(expected_sheet.render(scale=1.5)) as expected_bitmap:
                        image, expected_image = bitmap.to_pil().convert("RGB"), expected_bitmap.to_pil().convert("RGB")
                        assert image.size == expected_image.size
                        assert ImageChops.difference(image, expected_image).getbbox() is None, (name, index, "reference pixels")
                        image.save(folder / f"independent-page-{index + 1}.png")
                    with closing(sheet.get_textpage()) as text_page:
                        width, height = sheet.get_size()
                        for marker in markers:
                            with closing(text_page.search(marker, match_case=True)) as search:
                                found = search.get_next()
                                if found is not None:
                                    assert marker not in positions and search.get_next() is None
                                    left, _, _, top = text_page.get_charbox(found[0])
                                    positions[marker] = {"page": index + 1, "x": left, "y": height - top}
                        for char in range(text_page.count_chars()):
                            if text_page.get_text_range(char, 1).strip():
                                left, bottom, right, top = text_page.get_charbox(char)
                                assert -1 <= left <= right <= width + 1 and -1 <= bottom <= top <= height + 1, (name, index, char)
        assert set(positions) == set(markers)
        cases.append({"name": name, "pages": len(reader.pages), "pdf_sha256": sha256(pdf).hexdigest(),
                      "reference_sha256": sha256(reference_pdf).hexdigest(), "reference_bytes_equal": pdf == reference_pdf,
                      "reference_pixels_equal": True, "repeat_bytes_equal": True, "missing_glyphs": 0,
                      "positions": positions, "layout_events": trace["events"]})
        return positions

    def blocks():
        return [Box(Paragraph(marker), style=style(width=pt(width), height=pt(height), background=color))
                for marker, width, height, color in (("I_A", 48, 48, "#dce8f5"), ("I_B", 72, 24, "#e7eee4"))]

    Spaced = styled(Flow, row_gap=pt(12))
    for name, kwargs, expected in (("omitted", {}, 12), ("explicit", {"gap": pt(3)}, 3),
                                   ("explicit-builtin", {"gap": pt(8)}, 8), ("zero", {"gap": 0}, 0),
                                   ("style-final", {"gap": pt(3), "style": style(row_gap=pt(9))}, 9)):
        children = [Paragraph("I_A"), Paragraph("I_B")]
        points = check("flow-" + name, Spaced(children, **kwargs), Flow(children, gap=pt(expected)), ["I_A", "I_B"])
        # Check the logical gap separately from pixel-snapped text ink. Two
        # independently snapped origins can differ by almost one CSS pixel.
        # The 8 pt gap is fractional in CSS pixels; its pitch can be 19.5 pt
        # even though the two 12 pt line boxes plus that gap occupy 32 pt.
        placed = [event for event in cases[-1]["layout_events"] if event.get("result") == "placed"]
        assert len(placed) == 1, (name, placed)
        logical_height = placed[0]["placed_rect"]["h"]
        assert abs(logical_height - (24 + expected)) < .01, (name, logical_height)
        assert abs(points["I_B"]["y"] - points["I_A"]["y"] - (12 + expected)) < .751, (name, points)

    children = blocks()
    choices = {"align": "start", "justify": "start", "gap": pt(6), "style": style(width=pt(180))}
    points = check("row-alignment", styled(Row, align_items="end")(children, **choices), Row(children, **choices), ["I_A", "I_B"])
    assert abs(points["I_A"]["y"] - points["I_B"]["y"]) < .1
    points = check("row-justification", styled(Row, justify_content="end")(children, **choices), Row(children, **choices), ["I_A", "I_B"])
    # PDFium reports the ink box, including Inter's left sidebearing, rather
    # than the text origin at the 20 pt content edge.
    assert 20 <= points["I_A"]["x"] < 22, points
    points = check("row-gap", styled(Row, gap=pt(18))(children, **choices), Row(children, **choices), ["I_A", "I_B"])
    assert abs(points["I_B"]["x"] - points["I_A"]["x"] - 54) < .4
    choices = {"align": "start", "gap": pt(6), "style": style(width=pt(180))}
    points = check("stack-alignment", styled(Stack, align_items="end")(children, **choices), Stack(children, **choices), ["I_A", "I_B"])
    assert abs(points["I_A"]["x"] - points["I_B"]["x"]) < .1
    points = check("stack-gap", styled(Stack, gap=pt(18))(children, **choices), Stack(children, **choices), ["I_A", "I_B"])
    assert abs(points["I_B"]["y"] - points["I_A"]["y"] - 54) < .4

    markers = [f"I_{letter}" for letter in "ABCDEF"]
    cells = [Box(Paragraph(marker), style=style(height=pt(24), background="#dce8f5")) for marker in markers]
    choices = {"columns": 3, "gap": pt(6), "style": style(width=pt(180))}
    points = check("grid-columns", styled(Grid, grid_template_columns="1fr 1fr")(cells, **choices), Grid(cells, **choices), markers)
    assert abs(points["I_A"]["y"] - points["I_C"]["y"]) < .1
    assert points["I_D"]["y"] > points["I_A"]["y"] + 20
    choices = {"columns": None, "gap": pt(3), "style": style(width=pt(180))}
    points = check("grid-gap", styled(Grid, gap=pt(18))(cells, **choices), Grid(cells, **choices), markers)
    assert abs(points["I_C"]["y"] - points["I_A"]["y"] - 27) < .4

    small_page = Page((pt(200), pt(200)), margin=pt(20))
    for keep in (False, True):
        Title = styled(Heading, break_after="auto" if keep else "avoid")
        options = {"keep_with_next": keep, "style": style(font_size=pt(12), font_weight=400)}
        before = Box(Paragraph("I_PREV"), style=style(height=pt(114), break_inside="avoid"))
        after = Box(Paragraph("I_BODY"), style=style(height=pt(30), break_inside="avoid"))
        points = check("heading-kept" if keep else "heading-released",
                       Flow(before, Title("I_TITLE", **options), after, gap=pt(12)),
                       Flow(before, Heading("I_TITLE", **options), after, gap=pt(12)),
                       ["I_PREV", "I_TITLE", "I_BODY"], page=small_page)
        assert points["I_PREV"]["page"] == 1 and points["I_BODY"]["page"] == 2
        assert points["I_TITLE"]["page"] == (2 if keep else 1), (keep, points)

    @component
    def OptionalFlow(*, gap=None):
        return Flow(Paragraph("I_A"), Paragraph("I_B"), gap=gap)

    for value, expected in ((None, 12), (pt(3), 3)):
        check("forwarded-omitted" if value is None else "forwarded-explicit",
              styled(OptionalFlow, row_gap=pt(12))(gap=value), OptionalFlow(gap=pt(expected)), ["I_A", "I_B"])

    receipt = {"ok": True, "runtime": str(fullbleed.__file__), "cases": cases,
               "scope": "Installed explicit layout props, omitted/None choices, zero and false, final styles, optional forwarding and actual pagination. Independent PDF pixels match plain-component references; marker positions also check gaps, alignment, grid tracks and heading page placement."}
    (args.out / "verification.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases), "pages": sum(case["pages"] for case in cases)}))


if __name__ == "__main__":
    main()
