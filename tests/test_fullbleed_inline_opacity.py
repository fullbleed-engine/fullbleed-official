"""Inline effects preserve layout and composite actual PDF paint as one group."""
import io
import os
from pathlib import Path

import fullbleed
import fullbleed_assets
from pypdf import PdfReader
import pytest

pdfium = pytest.importorskip("pypdfium2")

CSS = """
@page { size: 220pt 180pt; margin: 12pt; }
body { font-family: 'Noto Sans'; font-size: 12pt; line-height: 18pt; }
p { margin: 0 0 12pt; }
.tile { display: inline-block; width: 40pt; height: 30pt; background: red; }
.second { margin-left: -20pt; }
.tight { line-height: 8pt; }
.tight b { font-size: 28pt; color: red; }
"""
LAYOUT_CASES = {
    "wrapping": '<p>Before <span class="fade">A long inline span with plain words that wraps across several lines without becoming an atomic box.</span> After.</p>',
    "styled": '<p>Before <span class="fade">A long inline span with <b style="font-weight:bold">bold words</b>, <i style="font-style:italic">italic words</i>, and ordinary text that wraps across several lines.</span> After.</p>',
    "pages": '<p>Before <span class="fade">' + "One two three four five six seven eight. " * 30 + '</span> After.</p>',
    "bidi": '<p style="direction:rtl;unicode-bidi:bidi-override">A <span class="fade">B C <b style="font-weight:bold">D E</b> F G H I J K L M N O P Q R S T U V W X Y Z</span> 1234</p>',
}


@pytest.mark.parametrize("separator", [" ", "\n  ", "<!-- formatting -->\n  ", "<span> </span>"])
def test_formatting_whitespace_preserves_collapsed_block_margins(separator):
    runtime = engine()
    css = CSS + ".block{margin:18pt 0;padding:10pt;border:1pt solid black}"
    first, second = '<div class="block">First</div>', '<div class="block">Second</div>'
    compact = bytes(runtime.render_pdf(first + second, css))
    indented = bytes(runtime.render_pdf(first + separator + second, css))
    assert geometry(compact) == geometry(indented)
    assert pixels(compact).tobytes() == pixels(indented).tobytes()


def engine():
    return fullbleed.PdfEngine(font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))])


def render(runtime, html, alpha, name):
    css = CSS + f".fade{{opacity:{alpha}}}"
    result = bytes(runtime.render_pdf(html, css))
    assert result == bytes(runtime.render_pdf(html, css)), "The same document must be deterministic"
    if directory := os.environ.get("FULLBLEED_INLINE_OPACITY_TEST_OUTPUT"):
        path = Path(directory)
        path.mkdir(parents=True, exist_ok=True)
        (path / f"{name}-{alpha}.pdf").write_bytes(result)
    return result


def geometry(pdf):
    document = pdfium.PdfDocument(pdf)
    pages = []
    for page in document:
        text = page.get_textpage()
        pages.append([
            (text.get_text_range(i, 1), tuple(text.get_charbox(i)))
            for i in range(text.count_chars()) if not text.get_text_range(i, 1).isspace()
        ])
        text.close()
        page.close()
    document.close()
    return pages


def pixels(pdf):
    document = pdfium.PdfDocument(pdf)
    page = document[0]
    bitmap = page.render(scale=2)
    image = bitmap.to_pil().convert("RGB")
    bitmap.close()
    page.close()
    document.close()
    return image


@pytest.mark.parametrize("case", list(LAYOUT_CASES))
@pytest.mark.parametrize("alpha", [0, 0.5])
def test_inline_opacity_preserves_glyph_positions_wrapping_and_pages(case, alpha):
    runtime = engine()
    expected = geometry(render(runtime, LAYOUT_CASES[case], 1, case))
    actual = geometry(render(runtime, LAYOUT_CASES[case], alpha, case))
    assert len(actual) == len(expected)
    for a, b in zip(actual, expected):
        assert [char for char, _ in a] == [char for char, _ in b]
        for (_, box), (_, control) in zip(a, b):
            assert box == pytest.approx(control, abs=0.001)


@pytest.mark.parametrize("case", list(LAYOUT_CASES))
def test_transparent_inline_wrapper_keeps_the_same_text_run_as_unwrapped_content(case):
    html = LAYOUT_CASES[case]
    assert html.count('<span class="fade">') == html.count('</span>') == 1
    unwrapped = html.replace('<span class="fade">', '').replace('</span>', '')
    runtime = engine()
    wrapped_pdf = render(runtime, html, 1, case)
    unwrapped_pdf = render(runtime, unwrapped, 1, case + '-unwrapped')
    wrapped, control = geometry(wrapped_pdf), geometry(unwrapped_pdf)
    assert len(wrapped) == len(control)
    for actual, expected in zip(wrapped, control):
        assert [char for char, _ in actual] == [char for char, _ in expected]
        for (_, bounds), (_, expected_bounds) in zip(actual, expected):
            assert bounds == pytest.approx(expected_bounds, abs=0.001)
    # Check the actual visible glyphs too, including synthesized bold/italic.
    assert pixels(wrapped_pdf).tobytes() == pixels(unwrapped_pdf).tobytes()


@pytest.mark.parametrize("case,html", [
    ("overlap", '<p><span class="fade"><i class="tile"></i><i class="tile second"></i></span></p>'),
    ("lines", '<p class="tight"><span class="fade"><b>Overlap</b><br><b>Overlap</b><br><b>Overlap</b></span></p>'),
])
def test_inline_opacity_composites_overlapping_children_and_lines_once(case, html):
    runtime = engine()
    opaque = pixels(render(runtime, html, 1, case))
    half = pixels(render(runtime, html, 0.5, case))
    points = [(x, y) for y in range(opaque.height) for x in range(opaque.width) if opaque.getpixel((x, y)) == (255, 0, 0)]
    assert len(points) > 100, "The specimen must contain enough solid ink to check overlap"
    for point in points:
        assert half.getpixel(point) == pytest.approx((255, 127, 127), abs=1)


@pytest.mark.parametrize("alpha", [0, 0.5, 1])
def test_block_inside_inline_preserves_surrounding_anonymous_text_lines(alpha):
    html = '<div>Before <span class="fade">First <div>Block inside an inline</div> Last</span> After</div>'
    runtime = engine()
    pages = geometry(render(runtime, html, alpha, "block-lines"))
    unwrapped = '<div>Before First<div>Block inside an inline</div>Last After</div>'
    control = geometry(render(runtime, unwrapped, 1, "block-lines-unwrapped"))
    assert len(pages) == 1
    chars = pages[0]
    text = "".join(char for char, _ in chars)
    assert text == "BeforeFirstBlockinsideaninlineLastAfter"
    boxes = {word: chars[text.index(word)][1] for word in ("Before", "First", "Block", "Last", "After")}
    # Capital glyphs differ slightly at the baseline; separate lines are 18pt.
    assert boxes["Before"][1] == pytest.approx(boxes["First"][1], abs=0.1)
    assert boxes["Last"][1] == pytest.approx(boxes["After"][1], abs=0.1)
    assert boxes["Before"][1] - boxes["Block"][1] == pytest.approx(18, abs=0.1)
    assert boxes["Block"][1] - boxes["Last"][1] == pytest.approx(18, abs=0.1)
    assert len(control) == 1 and len(control[0]) == len(chars)
    for (char, bounds), (expected_char, expected_bounds) in zip(chars, control[0]):
        assert char == expected_char
        assert bounds == pytest.approx(expected_bounds, abs=0.001), "A transparent inline wrapper must preserve glyph positions across a block boundary"


def test_nested_inline_opacity_applies_each_group_once():
    html = '<p><span class="fade"><i class="tile"></i><span style="opacity:0.5"><i class="tile"></i></span></span></p>'
    runtime = engine()
    opaque = pixels(render(runtime, html, 1, "nested"))
    half = pixels(render(runtime, html, 0.5, "nested"))
    counts = {"outer": 0, "inner": 0}
    for y in range(opaque.height):
        for x in range(opaque.width):
            color = opaque.getpixel((x, y))
            if color == (255, 0, 0):
                counts["outer"] += 1
                assert half.getpixel((x, y)) == pytest.approx((255, 127, 127), abs=1)
            elif color[0] == 255 and color[1] == color[2] and color[1] in (127, 128):
                # PDF readers can round 50% coverage to either adjacent byte.
                counts["inner"] += 1
                assert half.getpixel((x, y)) == pytest.approx((255, 191, 191), abs=1)
    assert all(count > 100 for count in counts.values())


@pytest.mark.parametrize("media", [
    '<svg width="36" height="24" viewBox="0 0 36 24" role="img" aria-label="Red icon"><rect width="36" height="24" fill="red"/></svg>',
    '<img alt="Red tile" width="40" height="30" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg=="/>',
])
def test_zero_inline_opacity_removes_text_image_paint_and_link_actions(media):
    html = f'<p><a class="fade" href="https://example.invalid/report">Hidden text {media}</a></p>'
    runtime = engine()
    name = "hidden-svg" if media.startswith("<svg") else "hidden-raster"
    opaque = pixels(render(runtime, html, 1, name))
    colors = opaque.getcolors(opaque.width * opaque.height)
    assert sum(count for count, pixel in colors if pixel == (255, 0, 0)) > 100
    pdf = render(runtime, html, 0, name)
    reader = PdfReader(io.BytesIO(pdf), strict=True)
    assert all(not page.get("/Annots") for page in reader.pages)
    assert pixels(pdf).getextrema() == ((255, 255), (255, 255), (255, 255))
