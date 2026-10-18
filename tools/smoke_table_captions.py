#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Saved-PDF caption ownership, reading order, record isolation and paint checks.

Development dependencies: pypdf and pypdfium2. Run an independent PDF/UA
validator separately; this structural regression suite is not certification.
"""
from __future__ import annotations

import argparse
from copy import copy
from hashlib import sha256
from importlib import metadata, resources
import io
import json
from pathlib import Path
import runpy

import fullbleed
from pypdf import PdfReader
from pypdf.generic import ArrayObject, ByteStringObject, ContentStream, NameObject, TextStringObject
import pypdfium2 as pdfium

_reader = runpy.run_path(str(Path(__file__).with_name('smoke_table_tagging.py')))
inspect_marked_content, identity, TEXT = (_reader[name] for name in ('inspect', 'identity', 'TEXT'))

PROFILES = ('tagged', 'pdfua1', 'pdfua2')
MODES = ('direct', 'compiled', 'repeat', 'reflow', 'compact')
CSS = '''@page {size:320pt 340pt;margin:18pt}
body {font-family:Inter;font-size:10pt;line-height:1.3} h1 {font-size:16pt;margin:0 0 10pt}
table,.table {width:100%;border-collapse:collapse} th,td,.cell {padding:5pt;text-align:left}
th,td {border:0.5pt solid #254d45} caption,.caption {padding:3pt;text-align:left}
.table {display:table} .caption {display:table-caption} .row {display:table-row} .cell {display:table-cell}
.bottom {caption-side:bottom} .hidden {display:none} .invisible {visibility:hidden}
.emptybox {height:15pt;background:#d8e8e0}'''


def documents():
    row = '<thead><tr><th id="item" scope="col">Item</th><th id="state" scope="col">Status</th></tr></thead><tbody><tr><td headers="item">CELL {{person}}</td><td headers="state">READY</td></tr></tbody>'
    for side in ('top', 'bottom'):
        yield side, '<table><caption class="'+side+'">CAP {{person}}</caption>'+row+'</table>', [('CAP {{person}}', side)]
    yield 'nested', '<table><caption>OUTER {{person}}</caption><tr><th scope="col">Detail</th></tr><tr><td><table><caption class="bottom">INNER {{person}}</caption>'+row+'</table></td></tr></table>', [('OUTER {{person}}', 'top'), ('INNER {{person}}', 'bottom')]
    yield 'adjacent', ''.join('<table><caption>CAP {{person}}</caption>'+row+'</table>' for _ in range(2)), [('CAP {{person}}', 'top')]*2
    rows = ''.join('<tr><td headers="item">ROW %02d {{person}}</td></tr>' % n for n in range(32))
    for side in ('top', 'bottom'):
        yield 'split-'+side, '<table><caption class="'+side+'">CAP {{person}}</caption><thead><tr><th id="item" scope="col">Item</th></tr></thead><tbody>'+rows+'</tbody></table>', [('CAP {{person}}', side)]
        words = 'CAP {{person}} ' + 'caption words ' * 180
        yield 'long-'+side, '<table><caption class="'+side+'">'+words+'</caption>'+row+'</table>', [(words.strip(), side)]
    for name, caption, expected in (
        ('empty', '', []),
        ('empty-box', ' class="emptybox"', []),
        ('display-none', ' class="hidden"', []),
        ('visibility-hidden', ' class="invisible"', []),
    ):
        text = 'HIDDEN' if 'hidden' in name or name == 'display-none' else ''
        yield name, '<table><caption'+caption+'>'+text+'</caption>'+row+'</table>', expected
    cssrow = '<div class="row"><div class="cell">CELL {{person}}</div><div class="cell">READY</div></div>'
    for side in ('top', 'bottom'):
        yield 'css-'+side, '<div class="table"><div class="caption '+side+'">CAP {{person}}</div>'+cssrow+'</div>', [('CAP {{person}}', side)]
    yield 'css-split', '<div class="table"><div class="caption">CAP {{person}}</div>'+cssrow*32+'</div>', [('CAP {{person}}', 'top')]


def pixels(data, folder=None):
    hashes = []
    with pdfium.PdfDocument(data) as document:
        for index in range(len(document)):
            page = document[index]
            bitmap = page.render(scale=2)
            hashes.append(sha256(bytes(bitmap.buffer)).hexdigest())
            if folder is not None:
                image = bitmap.to_pil()
                image.save(folder / f'page-{index+1}.png')
                image.close()
            bitmap.close()
            page.close()
    return hashes


def structure(data):
    reader = PdfReader(io.BytesIO(data))
    nodes = []
    def visit(value, parent=None, page=None):
        obj = value.get_object()
        if isinstance(obj, list):
            for child in obj:
                visit(child, parent, page)
        elif isinstance(obj, int):
            assert parent is not None and page is not None
            parent['content'].append((identity(page), int(obj)))
        elif isinstance(obj, dict):
            if obj.get('/Type') == '/MCR':
                visit(obj['/MCID'], parent, obj.get('/Pg', page))
            elif '/S' in obj:
                node = dict(id=identity(value), role=str(obj['/S']), parent=parent['id'] if parent else None,
                    children=[], content=[])
                nodes.append(node)
                if parent:
                    parent['children'].append(node['id'])
                if '/K' in obj:
                    visit(obj['/K'], node, obj.get('/Pg', page))
    visit(reader.trailer['/Root']['/StructTreeRoot']['/K'])
    by_id = {n['id']: n for n in nodes}
    def content(node):
        return node['content'] + [item for key in node['children'] for item in content(by_id[key])]
    def text_for(node):
        def original_bytes(value):
            # A content-stream string can look like PDFDocEncoding to pypdf,
            # while actually containing embedded-font glyph IDs. Preserve the
            # original bytes when serializing the filtered stream for extraction.
            if isinstance(value, TextStringObject):
                return ByteStringObject(value.original_bytes)
            if isinstance(value, list):
                return ArrayObject([original_bytes(item) for item in value])
            return value
        selected = set(content(node))
        parts = []
        for page in reader.pages:
            marks = []
            operations = []
            for operands, operator in ContentStream(page.get_contents(), reader).operations:
                if operator in {b'BDC', b'BMC'}:
                    props = operands[1] if operator == b'BDC' else {}
                    marks.append(props.get('/MCID') if isinstance(props, dict) else None)
                elif operator == b'EMC':
                    marks.pop()
                if operator not in TEXT or any((identity(page), mark) in selected for mark in marks):
                    operations.append((original_bytes(operands), operator))
            filtered = ContentStream(None, reader)
            filtered.operations = operations
            clone = copy(page)
            clone[NameObject('/Contents')] = filtered
            parts.append(clone.extract_text())
        return ' '.join(' '.join(parts).split())
    captions = []
    page_order = {identity(page): index for index, page in enumerate(reader.pages)}
    for node in nodes:
        if node['role'] != '/Caption':
            continue
        parent = by_id.get(node['parent'])
        siblings = parent['children'] if parent else []
        references = content(node)
        order = [(page_order[page], mcid) for page, mcid in references]
        captions.append(dict(id=node['id'], parent=node['parent'],
            parent_role=parent['role'] if parent else None,
            first=bool(siblings) and siblings[0] == node['id'],
            last=bool(siblings) and siblings[-1] == node['id'],
            text=text_for(node), content=references, content_in_page_order=order == sorted(order)))
    return dict(captions=captions, tables=[n['id'] for n in nodes if n['role'] == '/Table'])


def check(out: Path, baseline: Path | None = None) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    report = dict(schema='fullbleed.table_caption_smoke.v1', ok=False,
        versions={n: metadata.version(n) for n in ('fullbleed', 'pypdf', 'pypdfium2')},
        font_sha256=sha256(font.read_bytes()).hexdigest(), cases=[])
    for profile in PROFILES:
        engine = fullbleed.PdfEngine(pdf_profile=profile, document_lang='en-US', document_title='Table captions',
            font_files=[str(font)], document_timestamp='2026-10-01T00:00:00Z')
        plain_engine = fullbleed.PdfEngine(font_files=[str(font)], document_timestamp='2026-10-01T00:00:00Z')
        for name, body, expected in documents():
            html = '<h1>Table captions</h1>' + body
            direct_pixels = {}
            for person in ('Ada', 'Bea'):
                direct_pixels[person] = pixels(engine.render_pdf(html.replace('{{person}}', person), CSS))
            plain = bytes(plain_engine.render_pdf(html.replace('{{person}}', 'Ada'), CSS))
            plain_pixels = pixels(plain)
            for mode in MODES:
                folder = out/profile/name/mode
                folder.mkdir(parents=True)
                (folder/'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder/'source.css').write_text(CSS, encoding='utf-8', newline='\n')
                if mode == 'direct':
                    (folder/'untagged.pdf').write_bytes(plain)
                compiled = None if mode == 'direct' else engine.compile_pdf(
                    html if mode in {'reflow', 'compact'} else html.replace('{{person}}', 'Ada'), CSS)
                def render():
                    if mode == 'direct':
                        return bytes(engine.render_pdf(html.replace('{{person}}', 'Ada'), CSS))
                    if mode == 'compiled':
                        return bytes(compiled.render_pdf())
                    if mode == 'repeat':
                        return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({'person': ['Ada', 'Bea']},
                        compression='compact' if mode == 'compact' else 'throughput'))
                data = render()
                (folder/'document.pdf').write_bytes(data)
                result = inspect_marked_content(data)
                result.update(structure(data))
                errors = result['errors']
                people = ['Ada', 'Bea'] if mode in {'reflow', 'compact'} else ['Ada']*(2 if mode == 'repeat' else 1)
                wanted = [(text.replace('{{person}}', person), side) for person in people for text, side in expected]
                captions = result['captions']
                if len(captions) != len(wanted):
                    errors.append('Incorrect caption count')
                for caption, (text, side) in zip(captions, wanted):
                    if caption['parent_role'] != '/Table':
                        errors.append('Caption must be an immediate Table child')
                    if not caption['first' if side == 'top' else 'last']:
                        errors.append('Caption reading order differs from caption-side')
                    if caption['text'] != text:
                        errors.append('Caption text is missing, duplicated or assigned to another table')
                    if not caption['content_in_page_order']:
                        errors.append('Caption continuation reading order differs from page order')
                if len({tuple(c['parent']) for c in captions if c['parent']}) != len(captions):
                    errors.append('Several captions incorrectly share one table')
                expected_tables = (2 if name in {'nested', 'adjacent'} else 1)*len(people)
                if len(result['tables']) != expected_tables:
                    errors.append('Logical table count differs across pages or records')
                actual_pixels = pixels(data, folder if profile == 'pdfua2' and mode == 'direct' else None)
                if actual_pixels != [p for person in people for p in direct_pixels[person]]:
                    errors.append('Compiled/batch output differs from individual records')
                if mode == 'direct' and actual_pixels != plain_pixels:
                    errors.append('Tagging changes visible output')
                if data != render():
                    errors.append('Repeated output is not byte deterministic')
                if name.startswith(('split-', 'long-')) or name == 'css-split':
                    if len(result['pages']) <= len(people):
                        errors.append('Pagination fixture did not span pages')
                if baseline:
                    reference = baseline/profile/name/mode
                    old_data = (reference/'document.pdf').read_bytes()
                    if actual_pixels != pixels(old_data):
                        errors.append('Saved-PDF pixels differ from baseline runtime')
                    old_pages = PdfReader(io.BytesIO(old_data)).pages
                    if [p['text'] for p in result['pages']] != [p.extract_text() for p in old_pages]:
                        errors.append('Extracted page text differs from baseline runtime')
                    if mode == 'direct' and plain != (reference/'untagged.pdf').read_bytes():
                        errors.append('Untagged PDF bytes differ from baseline runtime')
                result.update(profile=profile, name=name, mode=mode, ok=not errors,
                    pdf_sha256=sha256(data).hexdigest(), pixels=actual_pixels)
                (folder/'inspection.json').write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
                report['cases'].append(result)
    report['ok'] = all(c['ok'] for c in report['cases'])
    (out/'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path)
    args = parser.parse_args()
    result = check(args.out, args.baseline)
    failures = [dict(profile=c['profile'], name=c['name'], mode=c['mode'], errors=sorted(set(c['errors'])))
        for c in result['cases'] if not c['ok']]
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']), failed=failures)))
    raise SystemExit(0 if result['ok'] else 1)
