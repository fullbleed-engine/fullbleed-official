"""Column spans use their complete resolved grid area (issue #156)."""

from io import BytesIO

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


@pytest.mark.parametrize("mode", ["direct", "compiled"])
@pytest.mark.parametrize("placement", ["span 6 / span 6", "1 / 7"])
def test_34_track_spans_fill_content_width(placement, mode):
    end = "span 28 / span 28" if placement.startswith("span") else "7 / 35"
    _check_spans("repeat(34, minmax(0, 1fr))", placement, end, 0, 400 * 6 / 34, mode)


@pytest.mark.parametrize("mode", ["direct", "compiled"])
@pytest.mark.parametrize(
    "tracks,gap,first_width",
    [
        ("1fr 2fr 3fr 4fr", 4, 120.4),
        ("40pt 1fr 2fr 80pt", 4, 40 + 4 + 268 / 3),
        ("40pt 80pt 120pt 160pt", 0, 120),
        ("repeat(4, minmax(0, 1fr))", 4, 198),
    ],
)
def test_spans_sum_resolved_tracks_and_gaps(tracks, gap, first_width, mode):
    _check_spans(tracks, "span 2", "span 2", gap, first_width, mode)


def _check_spans(tracks, first, second, gap, first_width, mode):
    html = '<html><body><div class="grid"><div class="a">A</div><div class="b">Fullbleed grid span</div></div></body></html>'
    css = f"""
        @page {{ size: 440pt 100pt; margin: 20pt }}
        body {{ margin: 0; font: 10pt/12pt Helvetica }}
        .grid {{ display: grid; grid-template-columns: {tracks}; gap: {gap}pt }}
        .grid > div {{ height: 40pt }}
        .a {{ grid-column: {first}; background: #ffcccc }}
        .b {{ grid-column: {second}; background: #ccccff }}
    """
    engine = fullbleed.PdfEngine()
    data = bytes(engine.render_pdf(html, css) if mode == "direct"
                 else engine.compile_pdf(html, css).render_pdf())
    reader = PdfReader(BytesIO(data))
    assert len(reader.pages) == 1
    # The wide second span must accommodate the complete phrase on one line.
    assert "Fullbleed grid span" in reader.pages[0].extract_text()
    document = pdfium.PdfDocument(data)
    try:
        page = document[0]
        bitmap = page.render(scale=2)
        try:
            image = bitmap.to_pil().convert("RGB")
            # Sample below the text across both backgrounds, away from edges.
            for x in range(21, 419):
                local = x - 20
                if abs(local - first_width) < 1 or abs(local - first_width - gap) < 1:
                    continue
                expected = ((255, 204, 204) if local < first_width else
                            (255, 255, 255) if local < first_width + gap else
                            (204, 204, 255))
                assert image.getpixel((x * 2, 50 * 2)) == expected, (x, tracks)
        finally:
            bitmap.close()
            page.close()
    finally:
        document.close()
