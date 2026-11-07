"""Independent PDF checks for the reduced cases in issues #157–160."""
from io import BytesIO

import fullbleed
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.cffLib import FDArrayIndex, FDSelect, FontDict
from pypdf import PdfReader
from PIL import Image
import pypdfium2 as pdfium
import pytest


def _font(path, *, upm=1000, cff=False, cid=False, weight=400):
    """Make original, license-free outlines with known dimensions."""
    builder = FontBuilder(upm, isTTF=not cff)
    space, letter = ("cid00001", "cid00002") if cid else ("space", "H")
    order = [".notdef", space, letter]
    builder.setupGlyphOrder(order)
    builder.setupCharacterMap({32: space, 72: letter})
    advance = round(upm * 0.8)
    glyphs = {}
    for name in order:
        pen = T2CharStringPen(advance, None) if cff else TTGlyphPen(None)
        if name == letter:
            x0, x1, top = round(upm * 0.1), round(upm * (0.4 + weight / 4000)), round(upm * 0.7)
            pen.moveTo((x0, 0))
            pen.lineTo((x1, 0))
            pen.lineTo((x1, top))
            pen.lineTo((x0, top))
            pen.closePath()
        glyphs[name] = pen.getCharString() if cff else pen.glyph()
    name = f"FeedbackSans-W{weight}"
    if cff:
        builder.setupCFF(name, {"FullName": name, "FamilyName": "Feedback Sans",
                               "FontMatrix": [1 / upm, 0, 0, 1 / upm, 0, 0]}, glyphs, {})
        if cid:
            top = builder.font["CFF "].cff.topDictIndex[0]
            top.ROS = ("Adobe", "Identity", 0)
            top.FontMatrix = [.001, 0, 0, .001, 0, 0]
            top.FDArray = FDArrayIndex()
            fd = FontDict()
            fd.Private = top.Private
            fd.FontMatrix = [1 / upm, 0, 0, 1 / upm, 0, 0]
            top.FDArray.append(fd)
            top.FDSelect = FDSelect(format=3)
            top.FDSelect.gidArray = [0] * len(order)
            top.CharStrings.fdArray = top.FDArray
            top.CharStrings.fdSelect = top.FDSelect
            del top.Private
    else:
        builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({name: (advance, round(upm * .1) if name == letter else 0)
                                    for name in order})
    builder.setupHorizontalHeader(ascent=round(upm * .8), descent=-round(upm * .2))
    builder.setupNameTable({"familyName": "Feedback Sans", "styleName": f"W{weight}",
                           "uniqueFontIdentifier": name, "fullName": name, "psName": name,
                           "typographicFamily": "Feedback Sans", "typographicSubfamily": f"W{weight}"})
    builder.setupOS2(sTypoAscender=round(upm * .8), sTypoDescender=-round(upm * .2),
                     usWinAscent=round(upm * .8), usWinDescent=round(upm * .2),
                     usWeightClass=weight)
    builder.setupPost()
    builder.save(path)
    return path


def _render(engine, html, css, mode):
    if mode == "direct":
        return bytes(engine.render_pdf(html, css))
    return bytes(engine.compile_pdf(html, css).render_pdf())


def _inspect(data):
    with pdfium.PdfDocument(data) as document:
        assert len(document) == 1
        page = document[0]
        text = page.get_textpage()
        bitmap = page.render(scale=2)
        try:
            chars = [(text.get_text_range(i, 1), text.get_charbox(i))
                     for i in range(text.count_chars())]
            return text.get_text_range(), chars, bitmap.to_pil().convert("RGB"), page.get_size()
        finally:
            bitmap.close()
            text.close()
            page.close()


@pytest.mark.parametrize("mode", ["direct", "compiled"])
@pytest.mark.parametrize("upm", [1000, 2048])
@pytest.mark.parametrize("cid", [False, True])
def test_cff_outline_scale_matches_truetype(tmp_path, mode, upm, cid):
    images = []
    for cff in [False, True]:
        path = _font(tmp_path / ("font.otf" if cff else "font.ttf"), cff=cff, cid=cff and cid, upm=upm)
        engine = fullbleed.PdfEngine(font_files=[str(path)])
        data = _render(engine, "<p>HH</p>", '''
            @page {size:200pt 100pt; margin:20pt} body {margin:0}
            p {margin:0; font:24pt "Feedback Sans"}
        ''', mode)
        pdf = tmp_path / ("cff.pdf" if cff else "ttf.pdf")
        pdf.write_bytes(data)
        text, _, image, _ = _inspect(data)
        image.save(pdf.with_suffix(".png"))
        assert text.strip() == "HH"
        bounds = image.convert("L").point(lambda x: 255 if x < 160 else 0).getbbox()
        assert bounds is not None
        assert 32 <= bounds[3] - bounds[1] <= 35, bounds
        images.append(bounds)
        previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages(str(pdf), 144)
        assert len(previews) == 1
        native = Image.open(BytesIO(bytes(previews[0]))).convert("L")
        native_bounds = native.point(lambda x: 255 if x < 160 else 0).getbbox()
        assert native_bounds is not None, "Native finalized-PDF preview lost the embedded glyphs"
        assert 32 <= native_bounds[3] - native_bounds[1] <= 35, native_bounds
    assert all(abs(a - b) <= 1 for a, b in zip(*images)), images


@pytest.mark.parametrize("mode", ["direct", "compiled"])
@pytest.mark.parametrize("weight,expected", [(400, 400), (450, 500), (500, 500), (600, 600), (700, 700)])
@pytest.mark.parametrize("reverse", [False, True])
def test_registered_family_selects_numeric_weight(tmp_path, mode, weight, expected, reverse):
    paths = [_font(tmp_path / f"w{w}.ttf", weight=w) for w in [400, 500, 600, 700]]
    engine = fullbleed.PdfEngine(font_files=[str(p) for p in paths[::(-1 if reverse else 1)]])
    data = _render(engine, "<p>HH</p>", f'''
        @page {{size:200pt 100pt; margin:20pt}} body {{margin:0}}
        p {{margin:0; font:24pt "Feedback Sans"; font-weight:{weight}}}
    ''', mode)
    (tmp_path / "weights.pdf").write_bytes(data)
    fonts = PdfReader(BytesIO(data)).pages[0]["/Resources"]["/Font"]
    names = [str(ref.get_object()["/BaseFont"]) for ref in fonts.values()]
    assert len(names) == 1 and f"FeedbackSans-W{expected}" in names[0], names
    assert _inspect(data)[0].strip() == "HH"


@pytest.mark.parametrize("mode", ["direct", "compiled"])
def test_multiline_alignment_and_block_strong_default(mode, tmp_path):
    data = _render(fullbleed.PdfEngine(), '''
        <table><tr><td>100<br>discount 50</td></tr><tr><td>100</td></tr></table>
        <div class="box">label<strong>VALUE</strong>note</div>
    ''', '''
        @page {size:360pt 240pt; margin:20pt}
        body {margin:0; font:12pt Helvetica}
        table {width:300px; border-collapse:collapse}
        td {border:1px solid black; padding:4px; text-align:right}
        .box {width:300px; text-align:center; border:1px solid black}
        strong {display:block}
    ''', mode)
    (tmp_path / "alignment.pdf").write_bytes(data)
    text, chars, image, _ = _inspect(data)
    image.save(tmp_path / "alignment.png")
    rows = {}
    for char, bounds in chars:
        if char.strip():
            rows.setdefault(round(bounds[1]), []).append(bounds)
    lines = sorted(rows.values(), key=lambda row: -row[0][1])
    assert len(lines) == 6, (text, rows)
    right_edges = [max(box[2] for box in line) for line in lines[:3]]
    assert max(right_edges) - min(right_edges) < 1, right_edges
    centers = [(min(box[0] for box in line) + max(box[2] for box in line)) / 2
               for line in lines[3:]]
    assert all(abs(center - 133.25) < 1 for center in centers), centers
    fonts = PdfReader(BytesIO(data)).pages[0]["/Resources"]["/Font"]
    assert any("Helvetica-Bold" in str(ref.get_object().get("/BaseFont", "")) for ref in fonts.values())


@pytest.mark.parametrize("mode", ["direct", "compiled"])
@pytest.mark.parametrize("markup", ["{text}", "<span>{text}</span>", "<p>{text}</p>"])
def test_hyphenated_invoice_cell_fits_printable_width(mode, markup, tmp_path):
    identifier = "INV-2026-1001 INV-2026-1001 INV"
    html = "<table><tr><td>" + markup.format(text=identifier) + "</td>" + '<td class="n">1,000,000</td>' * 6 + "</tr></table>"
    data = _render(fullbleed.PdfEngine(), html, '''
        @page {size:A4; margin:20mm} body {margin:0; font:12pt Helvetica}
        table {width:100%; border-collapse:collapse}
        td {border:1px solid black; padding:4px 8px}
        .n {white-space:nowrap; text-align:right} p {margin:0}
    ''', mode)
    (tmp_path / "invoice.pdf").write_bytes(data)
    text, chars, image, size = _inspect(data)
    image.save(tmp_path / "invoice.png")
    assert text.count("1,000,000") == 6, text
    extracted = PdfReader(BytesIO(data)).pages[0].extract_text()
    assert "".join(extracted.split()).startswith(identifier.replace(" ", "")), extracted
    margin = 20 * 72 / 25.4
    visible = [bounds for char, bounds in chars if char.strip()]
    assert max(box[2] for box in visible) <= size[0] - margin + .5
    # Ink, including the final cell border, must fit too: extractable text can
    # still be painted outside the page or clipped by a reader.
    bounds = image.convert("L").point(lambda x: 255 if x < 180 else 0).getbbox()
    assert bounds[2] / 2 <= size[0] - margin + 1, bounds
