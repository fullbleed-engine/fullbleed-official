"""Transparent wrappers preserve the item boundaries of their flex/grid parent."""
from importlib.resources import files
import re

import fullbleed
import pytest

pdfium = pytest.importorskip("pypdfium2")

CSS = """@page {size:600px 400px; margin:12px}
body {margin:0; font-family:Inter; font-size:20px; line-height:24px}
p {margin:0 0 12px}
.host {gap:12px; grid-template-columns:1fr 1fr; color:#175c52}
.contents {display:contents}
.inherit {display:inherit}
.generated::before {content:"["} .generated::after {content:"]"}
.item::before {content:"{"} .item::after {content:"}"}
"""

CASES = [
    ("generated-child", 'alpha<span class="item">beta</span>gamma',
     'alpha<span class="item">beta</span>gamma'),
    ("anonymous-words", 'alpha beta<span class="item">gamma delta</span>epsilon zeta',
     'alpha beta<span class="item">gamma delta</span>epsilon zeta'),
    ("inherited-contents", '<span class="inherit">alpha<span class="item">beta</span>gamma</span>',
     'alpha<span class="item">beta</span>gamma'),
    ("real-box-resets-context", '<div>alpha<span class="contents"><span class="item">beta</span>gamma</span></div>',
     '<div>alpha<span class="contents"><span class="item">beta</span>gamma</span></div>'),
]


def visible_positions(data):
    document = pdfium.PdfDocument(data)
    try:
        assert len(document) == 1
        page = document[0]
        try:
            text = page.get_textpage()
            try:
                visible = []
                coordinates = []
                for index in range(text.count_chars()):
                    char = text.get_text_range(index, 1)
                    if char and not char.isspace():
                        visible.append(char)
                        coordinates.extend(text.get_charbox(index))
                return "".join(visible), coordinates
            finally:
                text.close()
        finally:
            page.close()
    finally:
        document.close()


@pytest.mark.parametrize("layout", ["flex", "grid"])
@pytest.mark.parametrize("depth", [1, 3])
@pytest.mark.parametrize("name,body,flat_body", CASES, ids=[row[0] for row in CASES])
def test_contents_keeps_direct_child_item_geometry(tmp_path, layout, depth, name, body, flat_body):
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    wrapped = body
    for _ in range(depth - 1):
        wrapped = '<span class="contents">' + wrapped + '</span>'
    wrapped = '<span class="contents generated">' + wrapped + '</span>'
    flat = '<span>[</span>' + flat_body + '<span>]</span>'
    def document(content):
        return '<html><body><p>Record Ada</p><div class="host" style="display:' + layout + '">' + content + '</div></body></html>'

    reference = bytes(engine.render_pdf(document(flat), CSS))
    (tmp_path / "reference.pdf").write_bytes(reference)
    expected_text, expected_positions = visible_positions(reference)
    assert re.sub(r"[^a-zA-Z]", "", expected_text).startswith("RecordAda")
    html = document(wrapped)
    direct = bytes(engine.render_pdf(html, CSS))
    assert direct == bytes(engine.render_pdf(html, CSS))
    for mode, data in [
        ("direct", direct),
        ("compiled", bytes(engine.compile_pdf(html, CSS).render_pdf())),
    ]:
        (tmp_path / (mode + ".pdf")).write_bytes(data)
        actual_text, actual_positions = visible_positions(data)
        assert actual_text == expected_text, (name, layout, depth, mode)
        assert actual_positions == pytest.approx(expected_positions, abs=0.02), (name, layout, depth, mode)
