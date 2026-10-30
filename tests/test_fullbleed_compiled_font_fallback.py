"""Compiled records must match fresh documents when their selected fonts change."""
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


def engine():
    bundle = fullbleed.AssetBundle()
    for name, path in [
        ("Latin", files("fullbleed_assets").joinpath("fonts", "NotoSans-Regular.ttf")),
        ("CJK", Path(__file__).parent / "fixtures/table-wrapping/FullbleedCJKTest-Regular.ttf"),
    ]:
        bundle.add_file(str(path), fullbleed.AssetKind.Font, name=name, trusted=True)
    renderer = fullbleed.PdfEngine()
    renderer.register_bundle(bundle)
    return renderer


def inspect(data, directory, name):
    (directory / f"{name}.pdf").write_bytes(data)
    reader = PdfReader(BytesIO(data), strict=True)
    pages = []
    pdf = pdfium.PdfDocument(data)
    try:
        for index, parsed in enumerate(reader.pages):
            fonts = set()

            def visit(text, cm, tm, font, size):
                if text.strip() and font:
                    fonts.add(str(font.get("/BaseFont", "")).split("+")[-1])

            text = parsed.extract_text(visitor_text=visit)
            page = pdf[index]
            bitmap = page.render(scale=2)
            image = bitmap.to_pil().convert("RGB")
            image.save(directory / f"{name}-{index + 1}.png")
            pages.append(("".join(text.split()), fonts, image.size, image.tobytes()))
            bitmap.close()
            page.close()
    finally:
        pdf.close()
    (directory / f"{name}-readers.json").write_text(json.dumps([
        {"text": text, "fonts": sorted(fonts), "size": size}
        for text, fonts, size, _ in pages
    ], indent=2), encoding="utf-8")
    return pages


def compare_batches(renderer, html, css, values, tmp_path):
    expected = []
    for index, value in enumerate(values):
        data, missing = renderer.render_pdf_with_glyph_report(html.replace("{{name}}", value), css)
        assert not missing, "Test fonts must cover every character"
        pages = inspect(data, tmp_path, f"direct-{index}")
        assert len(pages) == 1
        expected.append(pages[0])

    template = renderer.compile_pdf(html, css)
    first = None
    for index, order in enumerate(([0, 1], [1, 0], [0, 1])):
        data = bytes(template.render_pdf_reflow_bindings({"name": [values[i] for i in order]}))
        pages = inspect(data, tmp_path, f"batch-{index}")
        assert len(pages) == len(order)
        for actual, record in zip(pages, order):
            reference = expected[record]
            assert actual[:3] == reference[:3], "Text, selected fonts, and page size must match"
            assert actual[3] == reference[3], "Saved PDF pixels must match fresh rendering"
        if index == 0:
            first = data
        elif index == 2:
            assert data == first, "Replaying a batch must reproduce the same PDF"
    return expected


@pytest.mark.parametrize("stack", ["Latin, CJK", "Latin"], ids=["authored-fallback", "registered-fallback"])
@pytest.mark.parametrize("style", ["normal", "italic"])
@pytest.mark.parametrize("reverse", [False, True], ids=["latin-first", "cjk-first"])
@pytest.mark.parametrize("html,values", [
    ("<p>{{name}}</p>", ["ABC", "東京"]),
    ("<p>Marker {{name}} end</p>", ["ASCII CONTROL", "Before 東京 after"]),
], ids=["whole-slot", "inline-slot"])
def test_compiled_reflow_reselects_fallback_fonts(tmp_path, stack, style, reverse, html, values):
    values = values[::-1] if reverse else values
    css = f"""@page {{ size:400pt 200pt; margin:12pt; }}
    body {{ font-family:{stack}; font-size:12pt; font-style:{style}; }}"""
    compare_batches(engine(), html, css, values, tmp_path)


@pytest.mark.parametrize("values", [["A", "B"], ["B", "A"]], ids=["primary-first", "fallback-first"])
def test_compiled_reflow_reselects_unicode_range_fonts(tmp_path, values):
    css = """@font-face { font-family:Ranged; src:url(Latin.ttf); unicode-range:U+0041; }
    @font-face { font-family:Ranged; src:url(CJK.ttf); unicode-range:U+0042; }
    @page { size:400pt 200pt; margin:12pt; }
    body { font-family:Ranged; font-size:12pt; }"""
    expected = compare_batches(engine(), "<p>{{name}}</p>", css, values, tmp_path)
    for value, (_, fonts, _, _) in zip(values, expected):
        assert fonts == ({"NotoSans-Regular"} if value == "A" else {"FullbleedCJKTest-Regular"})


@pytest.mark.parametrize("values", [["ABC", "Café"], ["Café", "ABC"]], ids=["covered-first", "missing-first"])
def test_compiled_reflow_handles_holes_in_a_fonts_metrics_range(tmp_path, values):
    css = """@page { size:400pt 200pt; margin:12pt; }
    body { font-family:CJK, Latin; font-size:12pt; font-variant-numeric:tabular-nums; }"""
    expected = compare_batches(engine(), "<p>{{name}}</p>", css, values, tmp_path)
    for value, (_, fonts, _, _) in zip(values, expected):
        assert fonts == ({"FullbleedCJKTest-Regular"} if value == "ABC"
                         else {"FullbleedCJKTest-Regular", "NotoSans-Regular"})
