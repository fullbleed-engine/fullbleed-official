"""Auto table columns must honor permitted text breaks without losing content."""
from io import BytesIO
from pathlib import Path

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


FONT = Path(__file__).parent / "fixtures/table-wrapping/FullbleedCJKTest-Regular.ttf"
JAPANESE = "東京都の交通計画と地域調査の結果を説明する日本語の文章です" * 3
LATIN = "ABCDEFGHIJKLMN" * 5


@pytest.mark.parametrize("markup", ["{text}", "<span>{text}</span>", "<p>{text}</p>"],
                         ids=["plain", "inline-element", "block-element"])
@pytest.mark.parametrize("text, wrapping, overflow", [
    (JAPANESE, "", False),
    (JAPANESE, "word-break: keep-all", True),
    (LATIN, "word-break: break-all", False),
    (LATIN, "overflow-wrap: anywhere", False),
    (LATIN, "line-break: anywhere", False),
    (LATIN, "overflow-wrap: break-word", True),
    (LATIN, "overflow-wrap: normal", True),
    (JAPANESE, "white-space: nowrap; overflow-wrap: anywhere", True),
    ("alpha beta gamma delta " * 8, "", False),
], ids=["cjk-normal", "cjk-keep-all", "break-all", "overflow-anywhere", "line-anywhere",
        "break-word-minimum", "unbreakable-latin", "nowrap", "word-separators"])
def test_auto_table_uses_wrappable_minimum(markup, text, wrapping, overflow):
    _check_table(markup, text, wrapping, overflow, "auto")


@pytest.mark.parametrize("text, wrapping", [(JAPANESE, ""), (LATIN, "overflow-wrap: anywhere")],
                         ids=["japanese", "latin"])
def test_fixed_table_wrapped_span_starts_inside_cell(text, wrapping):
    _check_table("<span>{text}</span>", text, wrapping, False, "fixed")


def _check_table(markup, text, wrapping, overflow, layout):
    html = "<html lang='ja'><body><table><tr><td>" + markup.format(text=text) + "</td></tr></table></body></html>"
    css = """
        @page { size: 360pt 360pt; margin: 24pt; }
        body { margin: 0; font: 12pt "Fullbleed CJK Test"; line-height: 1.5; }
        table { width: 180pt; border-collapse: collapse; }
        td { padding: 0; border: 1pt solid #175c52; }
        p { margin: 0; }
    """ + "td { " + wrapping + "; } table { table-layout: " + layout + "; }"
    engine = fullbleed.PdfEngine(font_files=[str(FONT)], document_lang="ja")
    pdf = engine.render_pdf(html, css)
    assert engine.render_pdf(html, css) == pdf
    reader = PdfReader(BytesIO(pdf), strict=True)
    assert len(reader.pages) == 1
    assert "".join(reader.pages[0].extract_text().split()) == "".join(text.split())

    with pdfium.PdfDocument(pdf) as document:
        page = document[0]
        textpage = page.get_textpage()
        try:
            boxes = [textpage.get_charbox(index) for index in range(textpage.count_chars())
                     if textpage.get_text_range(index, 1).strip()]
            assert boxes
            assert min(box[0] for box in boxes) >= 24 - 0.1
            assert min(box[1] for box in boxes) >= 24 - 0.1
            assert max(box[3] for box in boxes) <= 336 + 0.1
            right = max(box[2] for box in boxes)
            if overflow:
                assert right > 205 + 1, "Authored unbreakable content must retain its minimum"
            else:
                assert right <= 205 + 1, "Wrappable text escaped the authored table width"
        finally:
            textpage.close()
            page.close()


@pytest.mark.parametrize("markup, declaration, minimum", [
    ('<div class="target">FIRST</div>', 'width:240pt;', 265),
    ('<div class="target">FIRST</div>', 'min-width:240pt;', 265),
    ('<div><div class="target">FIRST</div></div>', 'width:240pt;', 265),
    ('<div><div class="target">FIRST</div></div>', 'min-width:240pt;', 265),
    ('<div class="target">FIRST</div>', 'width:240pt;box-sizing:border-box;padding:10pt;border:2pt solid #555;', 265),
    ('<div class="target">FIRST</div>', 'width:100%;', None),
], ids=['width', 'min-width', 'nested-width', 'nested-min-width', 'border-box', 'percentage'])
def test_auto_table_preserves_authored_child_widths(markup, declaration, minimum):
    html = '<table><tr><td>' + markup + '</td><td>SECOND</td></tr></table>'
    css = '''@page { size:600pt 400pt; margin:24pt; }
        body { margin:0; font:12pt Helvetica; }
        table { width:180pt; border-collapse:collapse; }
        td { padding:0; border:1pt solid #175c52; }
    ''' + '.target {' + declaration + '}'
    engine = fullbleed.PdfEngine()
    pdf = engine.render_pdf(html, css)
    assert engine.render_pdf(html, css) == pdf
    assert len(PdfReader(BytesIO(pdf), strict=True).pages) == 1
    with pdfium.PdfDocument(pdf) as document:
        page = document[0]
        text = page.get_textpage()
        try:
            contents = text.get_text_range()
            assert contents.count('FIRST') == contents.count('SECOND') == 1
            left = text.get_charbox(contents.index('SECOND'))[0]
            if minimum is not None:
                assert left >= minimum - 1, 'Next cell overlaps an explicitly sized child'
            else:
                right = text.get_charbox(contents.index('SECOND') + len('SECOND') - 1)[2]
                assert right <= 205 + 1, 'A cyclic percentage must not inflate the table minimum'
        finally:
            text.close()
            page.close()
