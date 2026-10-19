#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check ordinary running content against independently rendered records.

Development readers are optional test dependencies. Run external PDF/UA
validation separately; this matrix alone does not establish conformance.
"""
from __future__ import annotations

import argparse
from collections import Counter
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
MODES = ('throughput', 'compact')
CSS = '''@page {size:320pt 240pt;margin:50pt 20pt}
* {margin:0;box-sizing:border-box}
html,body {font-family:Inter;font-size:10pt;line-height:12pt}
h1,h2 {font-size:10pt;line-height:12pt}
section+section {break-before:page}
header {position:running(head)} footer {position:running(foot)}
@page {@top-left {content:element(head)} @bottom-left {content:element(foot)}}'''


def cases():
    values = ['Ada', 'Beatrice', 'Ada']
    yield 'body-control', '', '', values, True
    yield 'static-header', '<header>Static header</header>', '', values, True
    yield 'header-only-binding', '<header>{{person}}</header>', '', values, False
    yield 'shared-header', '<header>{{person}}</header>', '', values, True
    yield 'shared-footer', '<footer>{{person}}</footer>', '', values, True
    yield 'shared-header-and-footer', '<header>{{person}}</header><footer>{{person}}</footer>', '', values, True
    yield 'styled-shared-header', '<header><p><strong>{{person}}</strong></p></header>', 'header {color:#245541;padding:3pt;border:1pt solid #245541}', values, True
    yield 'first-page-suppressed', '<header>{{person}}</header>', '@page:first {@top-left {content:none}}', values, True
    yield 'empty-first-record', '<header>{{person}}</header>', '', ['', 'Beatrice', ''], True


def texts(data):
    return [' '.join(page.extract_text().split()) for page in PdfReader(io.BytesIO(data)).pages]


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    options = dict(font_files=[str(font)], document_lang='en-US', document_title='Running record values',
                   document_timestamp='2026-10-19T00:00:00Z')
    report = dict(schema='fullbleed.running_reflow_smoke.v1', ok=False, cases=[],
        versions={name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2')},
        font_sha256=sha256(font.read_bytes()).hexdigest())
    for profile in PROFILES:
        engine = fullbleed.PdfEngine(pdf_profile=profile, **options)
        for name, furniture, extra, values, body_binding in cases():
            value = '{{person}}' if body_binding else 'Body control'
            html = ('<!doctype html><html lang="en-US"><head><title>Running record values</title></head><body><main>'
                + furniture + '<section><h1>First</h1><p>' + value + '</p></section>'
                + '<section><h2>Second</h2><p>' + value + '</p></section></main></body></html>')
            css = CSS + extra
            direct = {person: bytes(engine.render_pdf(html.replace('{{person}}', person), css)) for person in set(values)}
            direct_texts = {person: texts(data) for person, data in direct.items()}
            direct_pixels = {person: pixels(data) for person, data in direct.items()}
            direct_roles = {person: inspect(data)['roles'] for person, data in direct.items()}
            expected_roles = Counter()
            for person in values:
                expected_roles.update({role: direct_roles[person].get(role, 0) for role in ('/H1', '/H2', '/P')})
            for mode in MODES:
                folder = out / profile / name / mode
                folder.mkdir(parents=True)
                (folder / 'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder / 'source.css').write_text(css, encoding='utf-8', newline='\n')
                for index, person in enumerate(values):
                    (folder / f'control-{index+1}.pdf').write_bytes(direct[person])
                compiled = engine.compile_pdf(html, css)

                def render():
                    return bytes(compiled.render_pdf_reflow_bindings({'person': values}, compression=mode))

                data = render()
                (folder / 'document.pdf').write_bytes(data)
                result = inspect(data)
                errors = result['errors']
                if len(result['pages']) != 2*len(values):
                    errors.append('Each record must retain two pages')
                actual_text = texts(data)
                expected_text = [text for person in values for text in direct_texts[person]]
                if actual_text != expected_text:
                    errors.append('Batch text differs from separately rendered records')
                actual_pixels = pixels(data, folder if profile == 'pdfua2' and mode == 'throughput' else None)
                if actual_pixels != [page for person in values for page in direct_pixels[person]]:
                    errors.append('Batch pixels differ from separately rendered records')
                for role, count in expected_roles.items():
                    if result['roles'].get(role, 0) != count:
                        errors.append('Body semantic ownership changed: '+role)
                if data != render():
                    errors.append('Repeated output is not byte deterministic')
                result.update(name=name, profile=profile, mode=mode, ok=not errors,
                    pdf_sha256=sha256(data).hexdigest(), pixels=actual_pixels,
                    expected_text=expected_text, actual_text=actual_text, records=values)
                (folder / 'inspection.json').write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
                report['cases'].append(result)
    report['ok'] = all(case['ok'] for case in report['cases'])
    (out / 'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    result = check(parser.parse_args().out)
    failed = [dict(name=c['name'], profile=c['profile'], mode=c['mode'], errors=c['errors'])
              for c in result['cases'] if not c['ok']]
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']), failed=failed)))
    raise SystemExit(0 if result['ok'] else 1)
