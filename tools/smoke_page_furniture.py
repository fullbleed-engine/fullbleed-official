#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check saved-PDF page furniture, reading order, counters and record isolation.

Development dependencies: pypdf and pypdfium2. Run an independent PDF/UA
validator separately; these fixture checks are not accessibility certification.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
import io
import json
from pathlib import Path
import runpy

import fullbleed
from pypdf import PdfReader

inspect = runpy.run_path(str(Path(__file__).with_name('smoke_table_tagging.py')))['inspect']
pixels = runpy.run_path(str(Path(__file__).with_name('smoke_table_captions.py')))['pixels']

PROFILES = ('tagged', 'pdfua1', 'pdfua2')
MODES = ('direct', 'compiled', 'repeat', 'reflow', 'compact')
CSS = '''@page {size:320pt 340pt;margin:40pt 24pt;font-size:9pt}
html,body {font-family:Inter;font-size:11pt;line-height:1.3;margin:0}
h1 {font-size:20pt;margin:0 0 12pt} h2 {font-size:15pt;margin:0 0 8pt}
p {margin:0 0 8pt} section+section {break-before:page}
header {position:running(report-head);font-size:9pt;color:#245541}
footer {position:running(report-foot);font-size:9pt;color:#245541}
header p,footer p {margin:0}'''


def documents():
    yield 'body-control', '', ''
    yield 'text-and-page-count', '', '''@page {
        @top-left {content:"Common Ground";background:#d8e8e0}
        @bottom-right {content:"FOOTER " counter(page) " / " counter(pages)} }'''
    yield 'running-rich-header', '<header><p><strong>Common Ground</strong> / {{person}}</p></header>', '''
        @page {@top-left {content:element(report-head)}}'''
    yield 'running-rich-footer', '<footer><p>Common Ground <strong>{{person}}</strong></p></footer>', '''
        @page {@bottom-left {content:element(report-foot)}}'''
    yield 'both-and-page-count', '<header><p><strong>Common Ground</strong> / {{person}}</p></header><footer><p>Community report</p></footer>', '''
        @page {@top-left {content:element(report-head)} @bottom-left {content:element(report-foot)}
        @bottom-right {content:"FOOTER " counter(page) " / " counter(pages)}}'''
    yield 'first-page-suppressed', '<header><p>Common Ground / {{person}}</p></header>', '''
        @page {@top-left {content:element(report-head)} @bottom-right {content:"FOOTER " counter(page) " / " counter(pages)}}
        @page:first {@top-left {content:none} @bottom-right {content:none}}'''
    yield 'named-string', '', '''h1,h2 {string-set:section content()}
        @page {@top-left {content:string(section)} @bottom-right {content:"FOOTER " counter(page) " / " counter(pages)}}'''
    boxes = ('top-left-corner', 'top-left', 'top-center', 'top-right', 'top-right-corner',
             'bottom-left-corner', 'bottom-left', 'bottom-center', 'bottom-right', 'bottom-right-corner',
             'left-top', 'left-middle', 'left-bottom', 'right-top', 'right-middle', 'right-bottom')
    yield 'all-margin-boxes', '', '@page {' + ''.join(
        f'@{box} {{content:"M{index:02d}";font-size:7pt;background:#e9eee5}}'
        for index, box in enumerate(boxes)) + '}'


def check(out: Path, baseline: Path | None = None) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    report = dict(schema='fullbleed.page_furniture_smoke.v1', ok=False,
        versions={n: metadata.version(n) for n in ('fullbleed', 'pypdf', 'pypdfium2')},
        font_sha256=sha256(font.read_bytes()).hexdigest(), cases=[])
    body = ''.join(f'<section><h{1 if i == 1 else 2}>Section {i} {{{{person}}}}</h{1 if i == 1 else 2}>'
        f'<p>BODY{i} {{{{person}}}}. Repeated furniture stays outside the reading order.</p></section>'
        for i in range(1, 4))
    for profile in PROFILES:
        options = dict(document_lang='en-US', document_title='Page furniture', font_files=[str(font)],
                       document_timestamp='2026-10-18T00:00:00Z')
        engine = fullbleed.PdfEngine(pdf_profile=profile, **options)
        plain_engine = fullbleed.PdfEngine(**options)
        for name, furniture, extra_css in documents():
            html = '<!doctype html><html lang="en-US"><head><title>Page furniture</title></head><body><main>' + furniture + body + '</main></body></html>'
            css = CSS + extra_css
            direct_pixels = {person: pixels(engine.render_pdf(html.replace('{{person}}', person), css)) for person in ('Ada', 'Bea')}
            plain = bytes(plain_engine.render_pdf(html.replace('{{person}}', 'Ada'), css))
            plain_pixels = pixels(plain)
            for mode in MODES:
                folder = out / profile / name / mode
                folder.mkdir(parents=True)
                (folder / 'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder / 'source.css').write_text(css, encoding='utf-8', newline='\n')
                if mode == 'direct':
                    (folder / 'untagged.pdf').write_bytes(plain)
                compiled = None if mode == 'direct' else engine.compile_pdf(
                    html if mode in {'reflow', 'compact'} else html.replace('{{person}}', 'Ada'), css)

                def render():
                    if mode == 'direct':
                        return bytes(engine.render_pdf(html.replace('{{person}}', 'Ada'), css))
                    if mode == 'compiled':
                        return bytes(compiled.render_pdf())
                    if mode == 'repeat':
                        return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({'person': ['Ada', 'Bea']},
                        compression='compact' if mode == 'compact' else 'throughput'))

                data = render()
                (folder / 'document.pdf').write_bytes(data)
                result = inspect(data)
                errors = result['errors']
                people = ['Ada', 'Bea'] if mode in {'reflow', 'compact'} else ['Ada'] * (2 if mode == 'repeat' else 1)
                if len(result['pages']) != 3 * len(people):
                    errors.append('Each record must retain three pages')
                for role, count in (('/H1', 1), ('/H2', 2), ('/P', 3)):
                    if result['roles'].get(role) != count * len(people):
                        errors.append('Furniture entered the structure or body content was lost: ' + role)
                for index, page in enumerate(result['pages']):
                    record, local = divmod(index, 3)
                    if record >= len(people):
                        continue
                    text = ' '.join(page['text'].split())
                    if f'BODY{local + 1} {people[record]}' not in text:
                        errors.append('Missing or incorrect body record text')
                    expected = f'FOOTER {local + 1} / 3'
                    if name in {'text-and-page-count', 'both-and-page-count', 'named-string'} or name == 'first-page-suppressed' and local:
                        if expected not in text:
                            errors.append('Page counter or total leaked across records')
                    if name == 'first-page-suppressed' and local == 0 and ('FOOTER' in text or 'Common Ground' in text):
                        errors.append('First-page furniture suppression was lost')
                    if name.startswith('running-rich') or name == 'both-and-page-count' or name == 'first-page-suppressed' and local:
                        if 'Common Ground' not in text:
                            errors.append('Running element was lost')
                    if name == 'all-margin-boxes' and any(f'M{i:02d}' not in text for i in range(16)):
                        errors.append('A page-margin box was lost')
                actual_pixels = pixels(data, folder if profile == 'pdfua2' and mode == 'direct' else None)
                if actual_pixels != [p for person in people for p in direct_pixels[person]]:
                    errors.append('Compiled/batch output differs from individual records')
                if mode == 'direct' and actual_pixels != plain_pixels:
                    errors.append('Tagging changes visible output')
                if data != render():
                    errors.append('Repeated output is not byte deterministic')
                if baseline:
                    old = (baseline / profile / name / mode / 'document.pdf').read_bytes()
                    if actual_pixels != pixels(old):
                        errors.append('Saved-PDF pixels differ from baseline')
                    if [p['text'] for p in result['pages']] != [p.extract_text() for p in PdfReader(io.BytesIO(old)).pages]:
                        errors.append('Extracted page text differs from baseline')
                result.update(profile=profile, name=name, mode=mode, ok=not errors,
                    pdf_sha256=sha256(data).hexdigest(), pixels=actual_pixels)
                (folder / 'inspection.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
                report['cases'].append(result)
    report['ok'] = all(c['ok'] for c in report['cases'])
    (out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path)
    args = parser.parse_args()
    result = check(args.out, args.baseline)
    failed = [dict(profile=c['profile'], name=c['name'], mode=c['mode'], errors=sorted(set(c['errors'])))
              for c in result['cases'] if not c['ok']]
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']), failed=failed)))
    raise SystemExit(0 if result['ok'] else 1)
