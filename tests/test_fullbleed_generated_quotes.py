"""Generated quotation marks follow document order in the saved PDF."""
from importlib.resources import files
from io import BytesIO
import re

import fullbleed
from pypdf import PdfReader
import pytest


CSS = '''@page { size: 600px 400px; margin: 12px }
body { margin: 0; font-family: Inter; font-size: 20px; quotes: "[" "]" "{" "}" }
.q::before { content: open-quote } .q::after { content: close-quote }
.open::before { content: no-open-quote } .after::after { content: no-open-quote }
.close::before { content: no-close-quote } .closeafter::after { content: no-close-quote }
.visibleclose::after { content: close-quote } .balanced::after { content: open-quote close-quote }
.suppressed::before { display: none }
'''

SCENARIOS = [
    ("nested", "q", 'outer<span class="q">inner</span>end', "", "[outer{inner}end]"),
    ("silent-before", "open", '<span class="q">one</span>', '<span class="q">two</span>', "{one}{two}"),
    ("silent-after", "after", '<span class="q">one</span>', '<span class="q">two</span>', "[one]{two}"),
    ("close-after-zero-probe", "closeafter", '<span class="open"></span>', '<span class="q">one</span>', "[one]"),
    ("visible-close-zero-probe", "visibleclose", '<span class="open"></span>', '<span class="q">one</span>', "][one]"),
    ("balanced-after", "balanced", '<span class="open"></span>', '<span class="q">one</span>', "{}{one}"),
]
CASES = [
    (f"{display}-{name}", f'<span class="{classes}" style="display:{display}">{children}</span>{after}', expected)
    for display in ["inline", "contents", "block", "inline-block"]
    for name, classes, children, after, expected in SCENARIOS
] + [
    ("display-none", '<span class="open" style="display:none"></span><span class="q">one</span>', "[one]"),
    ("visibility-hidden", '<span class="open" style="visibility:hidden"></span><span class="q">one</span>', "{one}"),
    ("no-negative-depth", '<span class="close"></span><span class="q">one</span>', "[one]"),
    ("repeated-last-pair", '<span class="q">outer<span class="q">inner<span class="q">deep</span></span></span>', "[outer{inner{deep}}]"),
    ("table-cells", '<table><tr><td class="q">one</td><td class="q">two</td></tr></table>', "[one][two]"),
    ("literal-control", '<span>[one]</span>', "[one]"),
    ("image-before", '<img class="open" width="1" height="1" alt=""><span class="q">one</span>', "[one]"),
    ("image-after", '<img class="after" width="1" height="1" alt=""><span class="q">one</span>', "[one]"),
    ("image-contents", '<img class="open" style="display:contents" alt=""><span class="q">one</span>', "[one]"),
    ("line-break-before", '<br class="open"><span class="q">one</span>', "[one]"),
    ("svg-before", '<svg xmlns="http://www.w3.org/2000/svg" class="open" width="1" height="1"></svg><span class="q">one</span>', "[one]"),
    ("rule-before", '<hr class="open"><span class="q">one</span>', "{one}"),
    ("input-before", '<input class="open" type="text" value=""><span class="q">one</span>', "[one]"),
    ("textarea-before", '<textarea class="open"></textarea><span class="q">one</span>', "[one]"),
    ("select-before", '<select class="open"></select><span class="q">one</span>', "[one]"),
    ("hidden-subtree", '<div style="display:none"><span class="open"></span></div><span class="q">one</span>', "[one]"),
    ("before-display-none", '<span class="open suppressed"></span><span class="q">one</span>', "[one]"),
]


@pytest.mark.parametrize("name,body,expected", CASES, ids=[case[0] for case in CASES])
def test_generated_quotes_in_saved_pdf_paths(tmp_path, name, body, expected):
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    html = '<html><body><p>Record {{name}}</p>' + body + '</body></html>'
    literal = html.replace("{{name}}", "Ada")
    direct = bytes(engine.render_pdf(literal, CSS))
    assert direct == bytes(engine.render_pdf(literal, CSS))
    compiled = engine.compile_pdf(literal, CSS)
    reflow = engine.compile_pdf(html, CSS)
    variants = [
        ("direct", direct, ["Ada"]),
        ("compiled", bytes(compiled.render_pdf()), ["Ada"]),
        ("repeat", bytes(compiled.render_pdf_batch(2)), ["Ada", "Ada"]),
        ("reflow", bytes(reflow.render_pdf_reflow_bindings({"name": ["Ada", "Bea"]})), ["Ada", "Bea"]),
    ]
    for mode, data, names in variants:
        (tmp_path / f"{name}-{mode}.pdf").write_bytes(data)
        reader = PdfReader(BytesIO(data))
        actual = re.sub(r"\s+", "", "".join(page.extract_text() or "" for page in reader.pages))
        assert len(reader.pages) == len(names), (name, mode)
        assert actual == "".join("Record" + value + expected for value in names), (name, mode, actual)


def test_quote_depth_does_not_leak_between_documents():
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    engine.render_pdf('<span class="open"></span><span class="q">first</span>', CSS)
    data = engine.render_pdf('<span class="q">second</span>', CSS)
    actual = re.sub(r"\s+", "", PdfReader(BytesIO(data)).pages[0].extract_text())
    assert actual == "[second]"
