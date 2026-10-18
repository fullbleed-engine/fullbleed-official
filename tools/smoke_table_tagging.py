#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Inspect saved table PDFs with independent content-stream and structure readers.

Development-only dependencies: pypdf and pypdfium2. This is a regression gate,
not a replacement for an external PDF/UA validator or screen-reader testing.
"""
from __future__ import annotations

import argparse
from collections import Counter
from hashlib import sha256
from importlib import metadata, resources
import io
import json
from pathlib import Path

import fullbleed
from pypdf import PdfReader
from pypdf.generic import ContentStream

PROFILES = ("tagged", "pdfua1", "pdfua2")
MODES = ("direct", "compiled", "repeat", "reflow", "compact")
TEXT = {b"Tj", b"TJ", b"'", b'"'}
GEOMETRY = {b"S", b"s", b"f", b"F", b"f*", b"B", b"B*", b"b", b"b*", b"sh"}
PAINT = TEXT | GEOMETRY | {b"Do"}
CSS = """@page {size:320pt 340pt;margin:18pt}
body {font-family:Inter;font-size:10pt;line-height:1.3}
h1 {font-size:16pt;margin:0 0 10pt} p {margin:0 0 4pt}
table {width:100%;border-collapse:collapse} th,td {padding:5pt;text-align:left}
.decorated th,.decorated td {border:1pt solid #254d45;background:#e9f1ed}
.bold td {font-weight:700} .accent {color:#a13241}
ul {margin:0;padding-left:14pt} svg {vertical-align:middle}
"""


def documents():
    cells = {
        "plain": "CELL {{person}}",
        "cell-css-bold": "CELL {{person}}",
        "span": "<span class='accent'>CELL {{person}}</span>",
        "strong": "<strong>CELL {{person}}</strong>",
        "paragraph": "<p>CELL {{person}}</p>",
        "mixed": "Before <strong>CELL {{person}}</strong> after",
        "list": "<ul><li>CELL {{person}}</li><li>Second item</li></ul>",
        "figure": "<p>CELL {{person}}</p><svg aria-label='Blue square diagram' width='18' height='18'><rect width='18' height='18' fill='#164b8b'/></svg>",
        "nested-table": "<table><caption>Detail</caption><tr><th scope='col'>Name</th></tr><tr><td><p>CELL {{person}}</p></td></tr></table>",
    }
    for name, cell in cells.items():
        classes = "bold" if name == "cell-css-bold" else "decorated" if name not in {"plain"} else ""
        yield name, ("<h1>Table content</h1><table class='"+classes+"'><caption>Programs</caption>"
            "<thead><tr><th scope='col'>Program</th><th scope='col'>Status</th></tr></thead>"
            "<tbody><tr><td>"+cell+"</td><td>READY</td></tr></tbody></table>")
    yield "spans-and-headers", """<h1>Table relationships</h1><table class='decorated'>
<caption>Program details</caption><thead><tr><th id='group' scope='col' colspan='2'><p>Program</p></th>
<th id='state' scope='col'><strong>Status</strong></th></tr></thead><tbody>
<tr><th id='person' scope='row' rowspan='2'><p>CELL {{person}}</p></th><td headers='group person'><span>Garden</span></td><td headers='state'>READY</td></tr>
<tr><td headers='group person'><p>Kitchen</p></td><td headers='state'><strong>OPEN</strong></td></tr>
</tbody></table>"""
    rows = "".join("<tr><td><p>ROW%02d <strong>CELL {{person}}</strong></p></td><td>READY</td></tr>" % i for i in range(32))
    yield "split-table", ("<h1>Program schedule</h1><table class='decorated'><caption>Schedule</caption>"
        "<thead><tr><th scope='col'><span>Program</span></th><th scope='col'>Status</th></tr></thead><tbody>"+rows+"</tbody></table>")
    yield "paragraph-control", "<h1>Paragraph control</h1><p>Before <strong>CELL {{person}}</strong> after.</p>"


def identity(value):
    reference = getattr(value, "indirect_reference", None)
    if reference is not None:
        return reference.idnum, reference.generation
    return (value.idnum, value.generation) if hasattr(value, 'idnum') else None


def inspect(data: bytes) -> dict:
    reader = PdfReader(io.BytesIO(data))
    errors = []
    pages = []
    owners = {}
    roles = Counter()
    cells = []
    figures = []
    ids = {}
    tree = reader.trailer["/Root"]["/StructTreeRoot"]

    def visit(value, parent=None, page=None, table=None):
        obj = value.get_object()
        if isinstance(obj, list):
            for item in obj:
                visit(item, parent, page, table)
        elif isinstance(obj, int):
            if page is None or parent is None:
                errors.append("Content reference lacks page or owning structure element")
            else:
                key = (identity(page), int(obj))
                if key in owners:
                    errors.append("Duplicate structure ownership: "+str(key))
                owners[key] = identity(parent)
        elif isinstance(obj, dict):
            if obj.get("/Type") == "/MCR":
                visit(obj["/MCID"], parent, obj.get("/Pg", page), table)
                return
            role = str(obj.get("/S", ""))
            if not role:
                return
            roles[role] += 1
            if role == "/Table":
                table = identity(value)
            if "/ID" in obj:
                key = str(obj["/ID"])
                if key in ids:
                    errors.append("Duplicate structure ID")
                ids[key] = (role, table)
            attributes = obj.get("/A", {})
            if hasattr(attributes, 'get_object'):
                attributes = attributes.get_object()
            if isinstance(attributes, list):
                attributes = next((a.get_object() for a in attributes if a.get_object().get("/O") == "/Table"), {})
            if role in {"/TH", "/TD"}:
                cells.append({"role": role, "table": table, "scope": str(attributes.get("/Scope", "")),
                    "columns": int(attributes.get("/ColSpan", 1)), "rows": int(attributes.get("/RowSpan", 1)),
                    "headers": [str(h) for h in attributes.get("/Headers", [])]})
            if role == "/Figure":
                figures.append(str(obj.get("/Alt", "")))
            if "/K" in obj:
                visit(obj["/K"], value, obj.get("/Pg", page), table)

    visit(tree["/K"])
    parent_tree = {}
    def parent_numbers(node):
        node = node.get_object()
        nums = node.get("/Nums", [])
        for offset in range(0, len(nums), 2):
            parent_tree[int(nums[offset])] = nums[offset+1].get_object()
        for child in node.get("/Kids", []):
            parent_numbers(child)
    parent_numbers(tree["/ParentTree"])
    observed = set()
    for index, page in enumerate(reader.pages):
        marks = []
        mcids = []
        paints = text_paints = artifacts = 0
        paint_program = []
        parent_entries = parent_tree.get(int(page["/StructParents"]), [])
        for operands, operator in ContentStream(page.get_contents(), reader).operations:
            if operator in {b"BDC", b"BMC"}:
                props = operands[1] if operator == b"BDC" else {}
                mcid = props.get("/MCID") if isinstance(props, dict) else None
                if mcid is not None:
                    mcid = int(mcid)
                    if any(item[1] is not None for item in marks):
                        errors.append(f"Nested MCID on page {index+1}: {mcid}")
                    if mcid in mcids:
                        errors.append(f"Repeated MCID on page {index+1}: {mcid}")
                    mcids.append(mcid)
                    key = (identity(page), mcid)
                    observed.add(key)
                    if mcid >= len(parent_entries) or owners.get(key) != identity(parent_entries[mcid]):
                        errors.append(f"Parent-tree ownership mismatch on page {index+1}: {mcid}")
                marks.append((operands[0] == "/Artifact", mcid, str(operands[0])))
            elif operator == b"EMC":
                if not marks:
                    errors.append("Unbalanced marked-content close")
                else:
                    marks.pop()
            else:
                paint_program.append([operator.decode('ascii'), [repr(o) for o in operands]])
                if operator in PAINT:
                    paints += 1
                    artifact = any(item[0] for item in marks)
                    covered = artifact or any(item[1] is not None for item in marks)
                    if not covered:
                        errors.append(f"Unmarked paint on page {index+1}: {operator!r}")
                    artifacts += int(artifact)
                    text_paints += int(operator in TEXT and not artifact)
                    if operator in GEOMETRY and any(item[2] in {"/TH", "/TD"} for item in marks) and not artifact:
                        # Figures and painted list labels are real content,
                        # unlike cell backgrounds and borders.
                        if not any(item[2] in {"/Figure", "/Lbl"} for item in marks):
                            errors.append(f"Cell decoration classified as real content on page {index+1}")
        if marks:
            errors.append("Unclosed marked-content sequences")
        if not paints or not text_paints:
            errors.append("Expected visible non-artifact text")
        pages.append({"mcids": mcids, "paint_count": paints, "artifact_paint_count": artifacts,
            "real_text_paint_count": text_paints, "text": page.extract_text(),
            "paint_sha256": sha256(json.dumps(paint_program).encode('utf-8')).hexdigest()})
    if set(owners) != observed:
        errors.append("Structure references and emitted MCIDs differ")
    for cell in cells:
        for header in cell['headers']:
            if ids.get(header) != ('/TH', cell['table']):
                errors.append("Header reference does not resolve to this table's TH")
    return {"ok": not errors, "errors": errors, "pages": pages, "roles": dict(roles),
            "cells": cells, "figures": figures}


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")))
    report = {"schema": "fullbleed.table_tagging_smoke.v1", "ok": False,
        "versions": {name: metadata.version(name) for name in ("fullbleed", "pypdf", "pypdfium2")},
        "font_sha256": sha256(font.read_bytes()).hexdigest(), "cases": []}
    for profile in PROFILES:
        engine = fullbleed.PdfEngine(pdf_profile=profile, document_lang="en-US",
            document_title="Table content regression", font_files=[str(font)])
        for name, html in documents():
            for mode in MODES:
                folder = out/profile/name/mode
                folder.mkdir(parents=True)
                (folder/'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder/'source.css').write_text(CSS, encoding='utf-8', newline='\n')
                compiled = None if mode == 'direct' else engine.compile_pdf(
                    html if mode in {'reflow','compact'} else html.replace('{{person}}', 'Ada'), CSS)
                def render():
                    if mode == 'direct':
                        return bytes(engine.render_pdf(html.replace('{{person}}', 'Ada'), CSS))
                    if mode == 'compiled':
                        return bytes(compiled.render_pdf())
                    if mode == 'repeat':
                        return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({'person':['Ada','Bea']},
                        compression='compact' if mode == 'compact' else 'throughput'))
                data = render()
                (folder/'document.pdf').write_bytes(data)
                result = inspect(data)
                errors = result['errors']
                if data != render():
                    errors.append('Repeated rendering is not byte deterministic')
                count = 1 if mode in {'direct', 'compiled'} else 2
                text = ''.join(p['text'] for p in result['pages'])
                if text.count('CELL Ada') != (32 if name == 'split-table' else 1) * (2 if mode == 'repeat' else 1):
                    errors.append('Missing or duplicate Ada cell content')
                if mode in {'reflow','compact'} and text.count('CELL Bea') != (32 if name == 'split-table' else 1):
                    errors.append('Missing or duplicate Bea cell content')
                if name != 'paragraph-control' and result['roles'].get('/TH', 0) < count:
                    errors.append('Table headers lost')
                if name == 'spans-and-headers':
                    cells = result['cells']
                    if sum(c['columns'] == 2 and c['scope'] == '/Column' for c in cells) != count:
                        errors.append('Column header span lost')
                    if sum(c['rows'] == 2 and c['scope'] == '/Row' for c in cells) != count:
                        errors.append('Row header span lost')
                    if sum(bool(c['headers']) for c in cells) != count * 4:
                        errors.append('Explicit header relationships lost')
                if name == 'figure' and result['figures'] != ['Blue square diagram'] * count:
                    errors.append('Figure alternative text lost')
                if name == 'split-table' and len(result['pages']) <= count:
                    errors.append('Split-table control did not cross a page')
                if profile == 'pdfua1' and mode == 'direct':
                    import pypdfium2 as pdfium
                    previews = []
                    with pdfium.PdfDocument(data) as pdf:
                        for i in range(len(pdf)):
                            page = pdf[i]
                            bitmap = page.render(scale=2)
                            image = bitmap.to_pil()
                            image.save(folder/f'page-{i+1}.png')
                            previews.append({'size': list(image.size), 'rgba_sha256': sha256(image.convert('RGBA').tobytes()).hexdigest()})
                            image.close(); bitmap.close(); page.close()
                    result['pdfium'] = previews
                result.update(name=name, profile=profile, mode=mode, ok=not errors,
                              pdf_sha256=sha256(data).hexdigest())
                report['cases'].append(result)
    report['ok'] = all(c['ok'] for c in report['cases'])
    (out/'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    report = check(args.out)
    print(json.dumps({'ok': report['ok'], 'cases': len(report['cases']),
        'failed': [{'profile': c['profile'], 'name': c['name'], 'mode': c['mode'], 'errors': sorted(set(c['errors']))}
                   for c in report['cases'] if not c['ok']]}))
    raise SystemExit(0 if report['ok'] else 1)
