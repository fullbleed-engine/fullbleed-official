"""Generated counter values follow authored order, independently of layout probes."""
from importlib.resources import files
from io import BytesIO
import re

import fullbleed
from pypdf import PdfReader
import pytest


CSS = '''@page { size: 600px 400px; margin: 12px }
* { margin: 0; padding: 0 }
body { font: 18px Inter; line-height: 1.5; counter-reset: n; quotes: "{" "}" }
.root::before { content: 'B[' counter(n) ']' }
.root::after { content: 'A[' counter(n) ']' }
.item { counter-increment: n }
.item::before { content: 'I[' counter(n) ']' }
.tail::before { content: 'T[' counter(n) ']' }
'''
BODY = ('<div class="root"><span class="item">First</span>'
        '<span class="item">Second</span></div><div class="tail">Tail</div>')
SCENARIOS = [
    ("after-read", "", "B[0]I[1]FirstI[2]SecondA[2]T[2]Tail"),
    ("after-increment", ".root::after { counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondA[5]T[5]Tail"),
    ("before-increment", ".root::before { counter-increment: n 3 }",
     "B[3]I[4]FirstI[5]SecondA[5]T[5]Tail"),
    ("both-increment", ".root::before { counter-increment: n 3 } .root::after { counter-increment: n 4 }",
     "B[3]I[4]FirstI[5]SecondA[9]T[9]Tail"),
    ("after-set", ".root::after { counter-set: n 8 }",
     "B[0]I[1]FirstI[2]SecondA[8]T[8]Tail"),
    ("before-reset", ".root::before { counter-reset: n 5 }",
     "B[5]I[1]FirstI[2]SecondA[2]T[2]Tail"),
    ("after-reset", ".root::after { counter-reset: n 9 }",
     "B[0]I[1]FirstI[2]SecondA[9]T[2]Tail"),
    ("empty-before", ".root::before { content: ''; counter-increment: n 3 }",
     "I[4]FirstI[5]SecondA[5]T[5]Tail"),
    ("empty-after", ".root::after { content: ''; counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondT[5]Tail"),
    ("quotes-and-counters", ".root::before { content: open-quote 'B[' counter(n) ']' } .root::after { content: 'A[' counter(n) ']' close-quote }",
     "{B[0]I[1]FirstI[2]SecondA[2]}T[2]Tail"),
]
CASES = [
    (f"{display}-{name}", BODY, f".root {{ display: {display} }}" + rules, expected)
    for display in ["block", "inline-block", "inline", "contents"]
    for name, rules, expected in SCENARIOS
] + [
    (f"{display}-before-reset-new-name", BODY,
     f".root {{ display: {display} }} body {{ counter-reset: none }} .root::before {{ counter-reset: n 5 }}",
     "B[5]I[6]FirstI[7]SecondA[7]T[0]Tail")
    for display in ["block", "inline-block", "inline"]
] + [
    ("hidden-subtree", BODY.replace('<span class="item">Second',
        '<span style="display:none"><span class="item">Hidden</span></span><span class="item">Second'),
     "", "B[0]I[1]FirstI[2]SecondA[2]T[2]Tail"),
    ("invisible-counter", BODY.replace('<span class="item">Second',
        '<span class="item" style="visibility:hidden">Hidden</span><span class="item">Second'),
     "", "B[0]I[1]FirstI[3]SecondA[3]T[3]Tail"),
    ("suppressed-pseudo", BODY,
     ".root::after { display: none; counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondT[2]Tail"),
    ("no-generated-box", BODY,
     ".root::after { content: none; counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondT[2]Tail"),
    ("nested-counter-stack", '<div class="root"><div class="item">First<div class="nest"><span class="item">Inner</span></div></div><span class="item">Second</span></div><div class="tail">Tail</div>',
     '.nest { counter-reset: n 5 } .nest::after { content: "N[" counters(n, ".") "]"; counter-increment: n 2 }',
     "B[0]I[1]FirstI[6]InnerN[1.8]I[2]SecondA[2]T[2]Tail"),
    ("table-cell-after", '<table><tr><td class="root"><span class="item">First</span><span class="item">Second</span></td><td class="tail">Tail</td></tr></table>',
     ".root::after { counter-increment: n 3 }", "B[0]I[1]FirstI[2]SecondA[5]T[5]Tail"),
    ("anonymous-cell-after", '<div style="display:table-row"><div style="display:table-cell" class="root">First</div><div style="display:table-cell" class="tail">Tail</div></div>',
     ".root::after { counter-increment: n 3 }", "B[0]FirstA[3]T[3]Tail"),
    ("replaced-counter", '<img alt="" width="1" height="1" style="counter-increment:n 4"><div class="tail">Tail</div>',
     "img::before { content: counter(n); counter-increment: n 99 }", "T[4]Tail"),
    ("counter-format", BODY,
     ".root::after { content: 'A[' counter(n, upper-roman) ']'; counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondA[V]T[5]Tail"),
    ("root-counter", BODY, "html { counter-reset: n 7 } body { counter-reset: none }",
     "B[7]I[8]FirstI[9]SecondA[9]T[9]Tail"),
    ("parent-counter-priority", '<div class="reset">First</div><div class="item">Second</div><div class="tail">End</div>',
     '.reset { counter-reset: n 5 } .reset::before { content: "R[" counter(n) "]" }',
     "R[5]FirstI[1]SecondT[1]End"),
    ("pagination", '<div class="root"><div class="item">First</div><div class="item next">Second</div></div><div class="tail">Tail</div>',
     ".next { break-before: page } .root::after { counter-increment: n 3 }",
     "B[0]I[1]FirstI[2]SecondA[5]T[5]Tail"),
    # These preserve Fullbleed's existing HTML list numbering. They are not
    # browser-parity assertions for Chromium's counter(list-item) behavior.
    ("list-start-value", '<ol start="3"><li>First</li><li value="8">Second</li><li>Third</li></ol>',
     "ol { list-style: none } li::before { content: counter(list-item) ':' }", "3:First8:Second9:Third"),
    ("list-value-whitespace", '<ol start=" 3 "><li>First</li><li value=" 8 ">Second</li><li>Third</li></ol>',
     "ol { list-style: none } li::before { content: counter(list-item) ':' }", "3:First8:Second9:Third"),
    ("list-reversed", '<ol reversed><li>First</li><li>Second</li><li>Third</li></ol>',
     "ol { list-style: none } li::before { content: counter(list-item) ':' }", "3:First2:Second1:Third"),
    ("literal-control", "<p>B[0]FirstA[2]</p>", "", "B[0]FirstA[2]"),
]


def normalized_text(data):
    reader = PdfReader(BytesIO(data))
    return len(reader.pages), re.sub(r"\s+", "", "".join(page.extract_text() or "" for page in reader.pages))


@pytest.mark.parametrize("name,body,rules,expected", CASES, ids=[case[0] for case in CASES])
def test_generated_counters_in_saved_pdf_paths(tmp_path, name, body, rules, expected):
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    html = '<html><body><p>Record {{name}}</p>' + body + '</body></html>'
    literal = html.replace("{{name}}", "Ada")
    css = CSS + rules
    direct = bytes(engine.render_pdf(literal, css))
    assert direct == bytes(engine.render_pdf(literal, css))
    compiled = engine.compile_pdf(literal, css)
    reflow = engine.compile_pdf(html, css)
    variants = [
        ("direct", direct, ["Ada"]),
        ("compiled", bytes(compiled.render_pdf()), ["Ada"]),
        ("repeat", bytes(compiled.render_pdf_batch(2)), ["Ada", "Ada"]),
        ("reflow", bytes(reflow.render_pdf_reflow_bindings({"name": ["Ada", "Bea"]})), ["Ada", "Bea"]),
    ]
    for mode, data, names in variants:
        (tmp_path / f"{name}-{mode}.pdf").write_bytes(data)
        pages, text = normalized_text(data)
        assert pages == (2 if name == "pagination" else 1) * len(names), (name, mode, pages)
        assert text == "".join("Record" + value + expected for value in names), (name, mode, text)


def test_counter_state_does_not_leak_between_documents():
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    engine.render_pdf(BODY, CSS + ".root::after { counter-increment: n 99 }")
    assert normalized_text(engine.render_pdf('<div class="tail">New</div>', CSS)) == (1, "T[0]New")
