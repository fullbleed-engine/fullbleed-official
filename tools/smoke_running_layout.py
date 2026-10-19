#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise the internal deferred page-margin layout path in saved PDFs.

This is a prerequisite for component page bands, not their public API. The fit
diagnostics checked here concern measured boxes, not arbitrary descendant ink.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata
import io
import json
from pathlib import Path
import runpy

import fullbleed
from pypdf import PdfReader

geometry = runpy.run_path(str(Path(__file__).with_name('smoke_page_margin_layout.py')))
inspect, pixels, rectangles = (geometry[k] for k in ('inspect', 'pixels', 'rectangles'))
PROFILES = ('tagged', 'pdfua1', 'pdfua2')
MODES = ('direct', 'compiled', 'repeat', 'reflow', 'compact')
BASE = '''@page {size:320pt 240pt;margin:80pt 20pt 40pt}
* {margin:0;box-sizing:border-box}
html,body {font-family:"Fullbleed Preview Mono";font-size:10pt;line-height:12pt}
h1,h2 {font-size:10pt;line-height:12pt} section+section {break-before:page}
.band {position:running(head);background:#d9ede3}'''


def band(text='Brand {{person}}', extra=''):
    return f'<div class="band {extra}" data-fb-running-layout="margin" data-fb-id="header">{text}</div>'


def cases():
    yield 'body-width', band('Portfolio update for {{person}}'), (
        '@page {@top-left {content:element(head);width:100pt}}'
        'body {width:40pt;padding:3pt}.band {padding:4pt}')
    yield 'named-page', band('Portfolio update and next steps for {{person}}'), (
        '@page {@top-left {content:element(head)}}'
        '@page narrow {size:180pt 240pt} section+section {page:narrow}.band {padding:4pt}')
    yield 'auto-neighbors', band('LEFT') + band('RIGHTRIGHT', 'right'), (
        '@page {@top-left {content:element(head)} @top-right {content:element(right)}}'
        '.right {position:running(right)}')
    yield 'suppress-first', band(), (
        '@page {@top-left {content:element(head)}} @page:first {@top-left {content:none}}')
    yield 'rich-footer', band('<h2>Brand {{person}}</h2><p>Record summary</p>'), (
        '@page {margin-bottom:60pt;@bottom-left {content:element(head);width:180pt}}'
        '.band {padding:4pt;border:1pt solid #24624e}')
    yield 'same-content-two-slots', band('Portfolio update for {{person}}'), (
        '@page {@top-left {content:element(head);width:90pt}'
        ' @top-right {content:element(head);width:150pt}}.band {padding:4pt}')
    yield 'shared-value', band('{{person}}'), '@page {@top-left {content:element(head);width:100pt}}'
    yield 'ordinary-shared-value', band('{{person}}').replace(' data-fb-running-layout="margin"', ''), (
        '@page {@top-left {content:element(head);width:100pt}}.band {width:100pt}')


def document(furniture, *, shared=False):
    sections = (
        '<section><h1>BODY1</h1><p>{{person}}</p></section>'
        '<section><h2>BODY2</h2><p>{{person}}</p></section>'
    ) if shared else (
        '<section><h1>First</h1><p>BODY1 <span>{{person}}</span></p></section>'
        '<section><h2>Second</h2><p>BODY2 <span>{{person}}</span></p></section>'
    )
    return ('<!doctype html><html lang="en-US"><head><title>Deferred margin layout</title></head>'
        '<body><main>' + furniture + sections + '</main></body></html>')


def expected_slots(name, local):
    if name == 'suppress-first' and local == 0:
        return 320, []
    if name == 'named-page' and local == 1:
        return 180, [(20, 140, 0, 80)]
    if name == 'auto-neighbors':
        left = 280 * 4 / 14
        return 320, [(20, left, 0, 80), (20 + left, 280 - left, 0, 80)]
    if name == 'rich-footer':
        return 320, [(20, 180, 180, 240)]
    if name == 'same-content-two-slots':
        return 320, [(20, 90, 0, 80), (150, 150, 0, 80)]
    return 320, [(20, 100 if name in {'body-width', 'shared-value', 'ordinary-shared-value'} else 280, 0, 80)]


def diagnostics(engine, out):
    folder = out / 'diagnostics'
    folder.mkdir()
    reports = []

    def check_error(name, action, expected):
        errors, payload, message = [], None, None
        try:
            action()
            errors.append('Expected native layout rejection')
        except ValueError as error:
            message = str(error)
            try:
                payload = json.loads(message.split('page-margin layout: ', 1)[1])
            except (IndexError, ValueError):
                errors.append('Missing structured margin-layout diagnostic')
            if payload is not None:
                for key, value in expected.items():
                    if payload.get(key) != value:
                        errors.append('Unexpected diagnostic field: ' + key)
                if payload.get('source_ids') != ['header']:
                    errors.append('Diagnostic lost component source identity')
                if payload.get('units') != 'millipoints':
                    errors.append('Diagnostic measurement units are missing')
        reports.append(dict(name=name, ok=not errors, errors=errors, message=message, payload=payload))

    html = document(band())
    css = BASE + '@page {@top-left {content:element(head);width:100pt}}'
    for name, extra, reason in (
        ('fixed-width', '.band {width:150pt}', 'measured_box_overflow'),
        ('fixed-height', '.band {height:90pt}', 'measured_box_overflow'),
        ('empty-slot', '@page {margin-top:0}', 'empty_slot'),
    ):
        check_error(name, lambda extra=extra: engine.render_pdf(html.replace('{{person}}', 'Ada'), css+extra),
            dict(schema='fullbleed.page_margin_fit.v1', code='layout.page_margin_size', reason=reason,
                 page=1, slot='top-left'))

    compiled = engine.compile_pdf(html, css)
    valid = {'person': ['Ada', 'Beatrice']}
    first = bytes(compiled.render_pdf_reflow_bindings(valid))
    invalid = {'person': ['Ada', 'words ' * 300]}
    expected = dict(reason='measured_box_overflow', page=1, record=2, slot='top-left')
    check_error('later-record-overflow', lambda: compiled.render_pdf_reflow_bindings(invalid), expected)
    # This native API is explicitly streaming; callers stage/rename when they
    # need atomic replacement. Check error propagation without promising that.
    destination = folder / 'failed-stream.pdf'
    check_error('streaming-file-reports-later-record-overflow',
        lambda: compiled.render_pdf_reflow_bindings_to_file(invalid, str(destination)), expected)
    reports[-1]['partial_file_bytes'] = destination.stat().st_size if destination.exists() else 0
    recovered = bytes(compiled.render_pdf_reflow_bindings(valid)) == first
    reports.append(dict(name='reuse-after-rejection', ok=recovered,
        errors=[] if recovered else ['Rejected binding poisoned subsequent output']))
    (folder / 'verification.json').write_text(json.dumps(reports, indent=2)+'\n', encoding='utf-8')
    return reports


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    font = Path(__file__).resolve().parents[1] / 'src/preview_fonts/FullbleedPreviewMono-Regular.ttf'
    options = dict(font_files=[str(font)], document_lang='en-US', document_title='Deferred margin layout',
        document_timestamp='2026-10-19T00:00:00Z')
    report = dict(schema='fullbleed.running_layout_smoke.v1', ok=False, cases=[], diagnostics=[],
        versions={name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2')},
        font_sha256=sha256(font.read_bytes()).hexdigest())
    for profile in PROFILES:
        engine = fullbleed.PdfEngine(pdf_profile=profile, **options)
        plain_engine = fullbleed.PdfEngine(**options)
        for name, furniture, extra in cases():
            html, css = document(furniture, shared=name.endswith('shared-value')), BASE + extra
            direct = {person: bytes(engine.render_pdf(html.replace('{{person}}', person), css))
                for person in ('Ada', 'Beatrice')}
            direct_pixels = {person: pixels(data) for person, data in direct.items()}
            for mode in MODES:
                folder = out / profile / name / mode
                folder.mkdir(parents=True)
                (folder/'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder/'source.css').write_text(css, encoding='utf-8', newline='\n')
                compiled = None if mode == 'direct' else engine.compile_pdf(
                    html if mode in {'reflow', 'compact'} else html.replace('{{person}}', 'Ada'), css)

                def render():
                    if mode == 'direct': return bytes(engine.render_pdf(html.replace('{{person}}', 'Ada'), css))
                    if mode == 'compiled': return bytes(compiled.render_pdf())
                    if mode == 'repeat': return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({'person': ['Ada', 'Beatrice']},
                        compression='compact' if mode == 'compact' else 'throughput'))

                data = render()
                (folder/'document.pdf').write_bytes(data)
                result = inspect(data)
                errors = result['errors']
                people = ['Ada', 'Beatrice'] if mode in {'reflow', 'compact'} else ['Ada']*(2 if mode == 'repeat' else 1)
                reader = PdfReader(io.BytesIO(data))
                if len(reader.pages) != 2 * len(people): errors.append('Each record must retain two pages')
                measured = []
                for index, page in enumerate(reader.pages):
                    record, local = divmod(index, 2)
                    if record >= len(people): continue
                    width, slots = expected_slots(name, local)
                    actual = rectangles(page, reader)
                    if abs(float(page.mediabox.width)-width) > .025: errors.append('Unexpected paper width')
                    if len(actual) != len(slots): errors.append('Missing or extra running-content rectangle')
                    for rect, slot in zip(actual, slots):
                        x, y, w, h = rect
                        left, available, top, bottom = slot
                        if abs(x-left) > .025 or abs(w-available) > .025:
                            errors.append('Running content did not use its final slot width')
                        if h <= 0 or y < top-.025 or y+h > bottom+.025:
                            errors.append('Measured running content escaped its margin slot')
                    text = ' '.join(page.extract_text().split())
                    person = people[record]
                    if f'BODY{local+1} {person}' not in text: errors.append('Body record text was lost')
                    occurrences = 1 + (len(slots) if name != 'auto-neighbors' else 0)
                    if text.count(person) != occurrences: errors.append('A header or body retained another record value')
                    if name == 'auto-neighbors' and not all(t in text for t in ('LEFT', 'RIGHTRIGHT')):
                        errors.append('Neighboring running text was lost')
                    measured.append(dict(page=index+1, slots=slots, rectangles=actual, text=text))
                for role, count in (('/H1', 1), ('/H2', 1), ('/P', 2)):
                    if result['roles'].get(role) != count*len(people): errors.append('Body reading order changed: '+role)
                raster = pixels(data, folder if profile == 'pdfua2' and mode == 'direct' else None)
                if raster != [p for person in people for p in direct_pixels[person]]:
                    errors.append('Record output differs visibly from independently rendered records')
                if mode == 'direct':
                    plain = bytes(plain_engine.render_pdf(html.replace('{{person}}', 'Ada'), css))
                    (folder/'untagged.pdf').write_bytes(plain)
                    if raster != pixels(plain): errors.append('Tagging changed visible layout')
                if data != render(): errors.append('Repeated output is not byte deterministic')
                result.update(profile=profile, name=name, mode=mode, ok=not errors,
                    pdf_sha256=sha256(data).hexdigest(), pixels=raster, measurements=measured)
                (folder/'inspection.json').write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
                report['cases'].append(result)
    report['diagnostics'] = diagnostics(fullbleed.PdfEngine(**options), out)
    report['ok'] = all(case['ok'] for case in (*report['cases'], *report['diagnostics']))
    (out/'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    result = check(parser.parse_args().out)
    failed = [dict(name=c['name'], profile=c.get('profile'), mode=c.get('mode'), errors=c['errors'])
        for c in (*result['cases'], *result['diagnostics']) if not c['ok']]
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']), diagnostics=len(result['diagnostics']), failed=failed)))
    raise SystemExit(0 if result['ok'] else 1)
