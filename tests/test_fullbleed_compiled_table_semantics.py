"""Installed-native regressions for table attributes across PDF emission paths.

These are structural checks, not a PDF/UA or screen-reader acceptance claim.
"""

import pytest

import fullbleed


HTML = """<table><thead><tr><th colspan='2' scope='col'>Items</th>
<th scope='col'>Price</th></tr></thead><tbody>
<tr><th rowspan='2' scope='row'>{{label}}</th><td>A</td><td>10</td></tr>
<tr><td>B</td><td>20</td></tr></tbody></table>"""
CSS = """@page {size:300pt 300pt;margin:12pt;}
table {width:100%;} th,td {padding:4pt;font:10pt Helvetica;}"""


@pytest.mark.parametrize("mode", ["direct", "compiled", "repeat", "fixed", "reflow", "compact"])
def test_compiled_table_spans_use_standard_attributes_in_every_pdf_path(mode):
    engine = fullbleed.PdfEngine(pdf_profile="tagged", document_lang="en-US")
    if mode == "direct":
        pdf = engine.render_pdf(HTML.replace("{{label}}", "Ada"), CSS)
    elif mode in {"compiled", "repeat"}:
        compiled = engine.compile_pdf(HTML.replace("{{label}}", "Ada"), CSS)
        pdf = compiled.render_pdf() if mode == "compiled" else compiled.render_pdf_batch(2)
    else:
        compiled = engine.compile_pdf(HTML, CSS)
        bindings = {"label": ["Ada", "Bea"]}
        if mode == "fixed":
            pdf = compiled.render_pdf_bindings(bindings)
        else:
            pdf = compiled.render_pdf_reflow_bindings(
                bindings, compression="compact" if mode == "compact" else "throughput"
            )
    records = 1 if mode in {"direct", "compiled"} else 2
    headers = [line for line in pdf.splitlines() if b"/Type /StructElem /S /TH " in line]
    assert len(headers) == records * 3
    for index in range(records):
        columns, price, row = headers[index * 3 : index * 3 + 3]
        assert b"/A << /O /Table /Scope /Column /ColSpan 2 >>" in columns
        assert b"/A << /O /Table /Scope /Column >>" in price
        assert b"/A << /O /Table /Scope /Row /RowSpan 2 >>" in row
    for line in headers:
        prefix, _, _attributes = line.partition(b" /A ")
        assert b"/Scope" not in prefix
        assert b"/ColSpan" not in prefix
        assert b"/RowSpan" not in prefix
    assert b"/Headers [" not in pdf, "No unsupported automatic header-reference guesses"
