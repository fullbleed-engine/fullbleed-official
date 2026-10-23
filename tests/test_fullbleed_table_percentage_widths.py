"""Percentage column wishes must not push fitting text outside a clipped table."""

from collections import Counter
from io import BytesIO
import re

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


@pytest.mark.parametrize("width_source", ["cells", "cols", "calc"])
@pytest.mark.parametrize("border_model", ["separate", "collapse"])
def test_percentage_columns_keep_text_inside_the_table(width_source, border_model):
    percentages = [26, 13, 19, 42]
    values = [f"calc({value}% + 0pt)" if width_source == "calc" else f"{value}%" for value in percentages]
    cols = "<colgroup>" + "".join(f'<col style="width:{value}">' for value in values) + "</colgroup>" if width_source == "cols" else ""
    headings = "".join(
        f'<th style="width:{value if width_source != "cols" else "auto"}">{label}</th>'
        for value, label in zip(values, ["Record", "Class", "Group", "Description"])
    )
    rows = "".join(
        f'<tr><td>ROW-{i:03}</td><td><span class="badge">INFORMATIONAL</span></td><td>General</td>'
        '<td>Illustrative report row with ordinary text and a second sentence for wrapping.</td></tr>'
        for i in range(1, 41)
    )
    html = f"<table>{cols}<thead><tr>{headings}</tr></thead><tbody>{rows}</tbody></table>"
    css = f"""
        @page {{ size: 600pt 300pt; margin: 20pt; }}
        * {{ box-sizing: border-box; }}
        body {{ margin: 0; font: 9pt Helvetica; }}
        table {{ width: 400pt; border: 1pt solid black; border-spacing: 0;
                 border-collapse: {border_model}; overflow: hidden; }}
        td, th {{ padding: 5pt; text-align: left; }}
        .badge {{ display: inline-block; padding: 2pt 6pt; letter-spacing: .4pt; }}
    """
    engine = fullbleed.PdfEngine()
    data = engine.render_pdf(html, css)
    assert data == engine.render_pdf(html, css)
    reader = PdfReader(BytesIO(data))
    assert len(reader.pages) > 1
    texts = [page.extract_text() for page in reader.pages]
    markers = re.findall(r"ROW-\s*(\d{3})", "\n".join(texts))
    assert Counter(markers) == Counter(f"{i:03}" for i in range(1, 41))
    assert all("Description" in text for text in texts if "ROW-" in text)
    document = pdfium.PdfDocument(data)
    try:
        for index in range(len(document)):
            page = document[index]
            text = page.get_textpage()
            try:
                for character in range(text.count_chars()):
                    if text.get_text_range(character, 1).strip():
                        left, bottom, right, top = text.get_charbox(character)
                        # Table border box is x=20..420pt. The PDF page is much
                        # wider, so page bounds alone would miss clipped text.
                        assert left >= 19.5 and right <= 420.5, (index, character, left, right)
                        assert bottom >= 19.5 and top <= 280.5, (index, character, bottom, top)
            finally:
                text.close()
                page.close()
    finally:
        document.close()
