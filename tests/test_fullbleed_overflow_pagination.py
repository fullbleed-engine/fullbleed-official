"""Clipped auto-height content must remain visible across physical PDF pages."""

from collections import Counter
from io import BytesIO
import re

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


def _assert_glyphs_inside_pages(pdf):
    document = pdfium.PdfDocument(pdf)
    try:
        for page_number in range(len(document)):
            page = document[page_number]
            text = page.get_textpage()
            try:
                width, height = page.get_size()
                for index in range(text.count_chars()):
                    if not text.get_text_range(index, 1).strip():
                        continue
                    left, bottom, right, top = text.get_charbox(index)
                    assert left >= -0.1 and bottom >= -0.1, (page_number, index, left, bottom)
                    assert right <= width + 0.1 and top <= height + 0.1, (page_number, index, right, top)
            finally:
                text.close()
                page.close()
    finally:
        document.close()


@pytest.mark.parametrize("overflow", ["hidden", "clip", "hidden visible", "visible hidden", "auto", "scroll"])
def test_auto_height_clipped_table_paginates_without_losing_rows(overflow):
    rows = "".join(f"<tr><td>ROW-{i:03}</td><td>Finding details</td></tr>" for i in range(1, 81))
    html = f"<h1>Report</h1><table><thead><tr><th>Record</th><th>Details</th></tr></thead><tbody>{rows}</tbody></table>"
    css = f"""
        @page {{ size: A5; margin: 15mm; }}
        body {{ font: 12pt Helvetica; }}
        table {{ width: 100%; border-collapse: separate; border-spacing: 0;
                 border: 1pt solid black; border-radius: 5pt; overflow: {overflow}; }}
        td, th {{ padding: 6pt; border-bottom: 0.5pt solid #999; }}
    """
    engine = fullbleed.PdfEngine()
    pdf = engine.render_pdf(html, css)
    reader = PdfReader(BytesIO(pdf))
    assert len(reader.pages) > 2
    texts = [page.extract_text() for page in reader.pages]
    markers = re.findall(r"ROW-\s*(\d{3})", "\n".join(texts))
    assert Counter(markers) == Counter(f"{i:03}" for i in range(1, 81))
    for text in texts:
        if "ROW-" in text:
            assert "Record" in text and "Details" in text
    _assert_glyphs_inside_pages(pdf)
    assert engine.render_pdf(html, css) == pdf


@pytest.mark.parametrize("height, pages", [("", 2), ("height: 120pt;", 1)])
def test_auto_height_clipping_honors_descendant_breaks_but_fixed_viewport_does_not(height, pages):
    html = '<div class="box"><p>BEFORE</p><p class="next">AFTER</p></div>'
    css = """
        @page { size: A5; margin: 15mm; }
        body { font: 12pt Helvetica; }
        p { height: 60pt; margin: 0; }
        .next { break-before: page; }
    """ + f".box {{ overflow: hidden; {height} }}"
    pdf = fullbleed.PdfEngine().render_pdf(html, css)
    reader = PdfReader(BytesIO(pdf))
    assert len(reader.pages) == pages
    texts = [page.extract_text() for page in reader.pages]
    assert "BEFORE" in texts[0] and "AFTER" in texts[-1]
    if pages == 2:
        assert "AFTER" not in texts[0] and "BEFORE" not in texts[1]
    _assert_glyphs_inside_pages(pdf)


def test_auto_height_clipping_keeps_preceding_line_when_footnote_call_moves():
    html = """<div class="lead"></div><section class="clip">
        <p>First line stays here<br>Second line has notes<span class="fn">first note</span><span class="fn">second note</span></p>
    </section>"""
    css = """
        @page { size: 192px 136px; margin: 8px;
                @footnote { border-top: 8px solid #111; padding-top: 8px; } }
        html { font-family: Helvetica; font-size: 12px; line-height: 20px; }
        * { margin: 0; box-sizing: border-box; }
        .lead { width: 176px; height: 72px; }
        p { width: 176px; line-height: 20px; }
        .clip { overflow: hidden; }
        .fn { float: footnote; footnote-display: inline; footnote-policy: line; }
    """
    pdf = fullbleed.PdfEngine().render_pdf(html, css)
    reader = PdfReader(BytesIO(pdf))
    assert len(reader.pages) == 2
    first, second = [page.extract_text() for page in reader.pages]
    assert "First line stays here" in first and "Second line has notes" not in first
    assert "First line stays here" not in second and "Second line has notes" in second
    assert "first note" in second and "second note" in second
    _assert_glyphs_inside_pages(pdf)
