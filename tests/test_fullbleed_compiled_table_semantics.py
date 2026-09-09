"""Installed-native regressions for table attributes across PDF emission paths.

These are structural checks, not a PDF/UA or screen-reader acceptance claim.
"""

import re

import pytest

import fullbleed


HTML = """<table><thead><tr><th colspan='2' scope='col'>Items</th>
<th scope='col'>Price</th></tr></thead><tbody>
<tr><th rowspan='2' scope='row'>{{label}}</th><td>A</td><td>10</td></tr>
<tr><td>B</td><td>20</td></tr></tbody></table>"""
CSS = """@page {size:300pt 300pt;margin:12pt;}
table {width:100%;} th,td {padding:4pt;font:10pt Helvetica;}"""


@pytest.mark.parametrize("mode", ["direct", "compiled", "repeat", "reflow", "compact"])
def test_compiled_table_spans_use_standard_attributes_in_supported_tagged_pdf_paths(mode):
    engine = fullbleed.PdfEngine(pdf_profile="tagged", document_lang="en-US")
    if mode == "direct":
        pdf = engine.render_pdf(HTML.replace("{{label}}", "Ada"), CSS)
    elif mode in {"compiled", "repeat"}:
        compiled = engine.compile_pdf(HTML.replace("{{label}}", "Ada"), CSS)
        pdf = compiled.render_pdf() if mode == "compiled" else compiled.render_pdf_batch(2)
    else:
        compiled = engine.compile_pdf(HTML, CSS)
        bindings = {"label": ["Ada", "Bea"]}
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


def test_tagged_fixed_geometry_binding_refusal_remains_explicit():
    bindings = {"label": ["Ada", "Bea"]}
    tagged = fullbleed.PdfEngine(pdf_profile="tagged").compile_pdf(HTML, CSS)
    assert tagged.stats()["binding_slots"] == ["label"]
    with pytest.raises(ValueError, match="compiled fixed-geometry bindings do not yet support tagged page structure"):
        tagged.render_pdf_bindings(bindings)
    # The refusal concerns tagging, not a new span/record limit or broken data
    # binding. Do not silently replace the requested profile with untagged PDF.
    plain = fullbleed.PdfEngine().compile_pdf(HTML, CSS).render_pdf_bindings(bindings)
    assert plain.count(b"/Type /Page ") == 2
    assert b"Ada" in plain and b"Bea" in plain
    assert b"/StructTreeRoot" not in plain


@pytest.mark.parametrize("mode", ["direct", "compiled", "repeat", "reflow", "compact"])
def test_explicit_table_headers_are_record_scoped_across_supported_compiled_paths(mode):
    html = """<table><thead><tr><th id='item'>Item</th><th id='price'>Price</th></tr></thead>
    <tbody><tr><td headers='item'>{{label}}</td><td headers='item price'>12</td></tr></tbody></table>"""
    engine = fullbleed.PdfEngine(pdf_profile="tagged", document_lang="en-US")

    def render():
        if mode == "direct":
            return engine.render_pdf(html.replace("{{label}}", "Ada"), CSS)
        compiled = engine.compile_pdf(html if mode in {"reflow", "compact"} else html.replace("{{label}}", "Ada"), CSS)
        if mode == "compiled":
            return compiled.render_pdf()
        if mode == "repeat":
            return compiled.render_pdf_batch(3)
        return compiled.render_pdf_reflow_bindings(
            {"label": ["Ada", "Bea", "Cy"]},
            compression="compact" if mode == "compact" else "throughput",
        )

    pdf = render()
    assert pdf == render(), "Global layout counters must not become persisted identities"
    count = 1 if mode in {"direct", "compiled"} else 3
    ids = re.findall(rb"/S /TH [^\r\n]* /ID \(([^)]+)\)", pdf)
    assert len(ids) == count * 2 and len(set(ids)) == len(ids)
    cells = [line for line in pdf.splitlines() if b"/S /TD " in line]
    for index in range(count):
        item, price = ids[index * 2 : index * 2 + 2]
        assert b"/Headers [(" + item + b")]" in cells[index * 2]
        assert b"/Headers [(" + item + b") (" + price + b")]" in cells[index * 2 + 1]
    assert b"/IDTree " in pdf


def test_table_header_trace_reports_unresolved_links_without_source_ids_or_record_text():
    html = """<table><tr><th id='private-header-id'>Private header text</th>
    <td headers='private-header-id missing-private-id'>Private recipient text</td></tr></table>"""
    engine = fullbleed.PdfEngine(pdf_profile="tagged")
    trace = engine.export_render_time_structure_trace(html, CSS)
    assert trace["summary"]["explicit_table_header_cell_count"] == 1
    assert trace["summary"]["resolved_table_header_link_count"] == 1
    assert trace["summary"]["table_header_issue_counts"] == {"missing_header_target": 1}
    assert "Private" not in str(trace) and "private-id" not in str(trace) and "private-header-id" not in str(trace)
    semantics = [event["table_semantics"] for page in trace["pages"] for event in page["sample_events"] if "table_semantics" in event]
    assert any(node["header_issues"] == ["missing_header_target"] for node in semantics)


def test_installed_runtime_advertises_compiled_table_relationships():
    features = fullbleed.build_features()
    assert features["explicit_table_headers"] is True
    assert features["logical_table_pagination"] is True
