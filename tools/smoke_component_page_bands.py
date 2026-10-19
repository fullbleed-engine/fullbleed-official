#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Save and inspect component bands across PDF profiles and record reflow.

Independent reader checks do not replace external PDF/UA validation or human
accessibility review. CI retains the actual output and source for both.
"""
from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import replace
from hashlib import sha256
from importlib import metadata, resources
import io
import json
from pathlib import Path
import runpy

from pypdf import PdfReader
from fullbleed.ui.experimental import (
    Document, Heading, Node, Page, PageBand, PageBandError, PageNumbers,
    Paragraph, Renderer, Stack, Style, Text, Theme, component, css, pt,
)
from fullbleed.ui.experimental.bootstrap import Badge

INSPECT = runpy.run_path(str(Path(__file__).with_name('smoke_table_tagging.py')))['inspect']
PIXELS = runpy.run_path(str(Path(__file__).with_name('smoke_table_captions.py')))['pixels']
THEME = Theme(text=Style(font_family='Inter', font_size=pt(10), line_height=pt(12)))


@component
def Brand():
    return Stack(Text('FIELD NOTES'), Badge('READY', variant='light'), gap=pt(3)).with_style(font_size=pt(8))


def document(*, footer=None, header=None, styles=None, blank=False):
    children = tuple(Node('section', (Heading(title, level=1 if index == 0 else 2), Paragraph('Body content.')),
                          style=Style(break_after='right' if blank else 'page'))
                     for index, title in enumerate(('First', 'Second') if blank else ('First', 'Second', 'Third')))
    return Document(*children, title='Component page bands', lang='en-US', theme=THEME, styles=styles,
                    page=Page((pt(360), pt(260)), (pt(45), pt(20), pt(45), pt(20)), header=header, footer=footer))


def cases():
    yield 'rich-header', document(header=PageBand(left=Brand(), right='Review copy'),
                                 footer=PageBand(left='Record', right=PageNumbers(total=True))), [1, 2, 3]
    yield 'numbers-only', document(footer=PageBand(right=PageNumbers(total=True))), [1, 2, 3]
    yield 'styled-numbers', document(footer=PageBand(right=PageNumbers(total=True, class_name='folio')),
        styles='.folio {font:italic 700 8pt/10pt Inter;color:#183e34;background:#edf2e6;margin:6pt;padding:2pt 4pt;border:1pt solid #53665c}'), [1, 2, 3]
    yield 'first-suppressed', document(footer=PageBand(right=PageNumbers(total=True), first_page=False)), [2, 3]
    yield 'blank-suppressed', document(footer=PageBand(right=PageNumbers(total=True)), blank=True), [1, 3]
    yield 'blank-visible', document(footer=PageBand(right=PageNumbers(total=True), blank_pages=True), blank=True), [1, 2, 3]
    yield 'hidden-numbers', document(footer=PageBand(right=PageNumbers(total=True, class_name='folio')),
                                    styles='.folio {display:none}'), []


def texts(data):
    return [' '.join(page.extract_text().split()) for page in PdfReader(io.BytesIO(data)).pages]


def save_source(doc, folder):
    plan = doc.compile(a11y_mode='raise', target='pdf')
    (folder / 'source.html').write_text(plan.html, encoding='utf-8', newline='\n')
    (folder / 'source.css').write_text(plan.css, encoding='utf-8', newline='\n')
    for styled, name in ((True, 'browser.html'), (False, 'browser-unstyled.html')):
        (folder / name).write_text(doc.to_html(include_styles=styled, a11y_mode='raise'), encoding='utf-8', newline='\n')
    return plan


def inspect_saved(data, folder, *, preview, blank_pages=()):
    (folder / 'document.pdf').write_bytes(data)
    result = INSPECT(data)
    # The shared table inspector expects body text on every page. Permit only
    # the exact, intentional blank pages in these fixtures; never mask a missing
    # body on any other page or any structural diagnostic.
    empty = {index for index, page in enumerate(result['pages'], 1) if not page['real_text_paint_count']}
    expected = set(blank_pages)
    if expected:
        message = 'Expected visible non-artifact text'
        if empty == expected and result['errors'].count(message) == len(expected):
            result['errors'] = [error for error in result['errors'] if error != message]
        else:
            result['errors'].append('Intentional blank pages do not match saved-PDF body content')
    result['intentional_blank_pages'] = list(blank_pages)
    result.update(pdf_sha256=sha256(data).hexdigest(), pixels=PIXELS(data, folder if preview else None))
    return result


def record_document(person, message):
    return Document(Heading('Record report', level=1), Paragraph(message), title='Record page bands', lang='en-US', theme=THEME,
                    page=Page((pt(360), pt(260)), pt(45), header=PageBand(left=person),
                              footer=PageBand(right=PageNumbers(total=True))))


def check(out):
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    report = dict(schema='fullbleed.component_page_bands_smoke.v1', ok=False, cases=[],
                  versions={name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2')},
                  font_sha256=sha256(font.read_bytes()).hexdigest(),
                  scope='Saved fixture structure, text, pixels, byte replay and record behavior; external conformance validation is separate.')
    for profile in ('tagged', 'pdfua1', 'pdfua2'):
        renderer = Renderer(font_files=[str(font)], pdf_profile=profile, document_timestamp='2026-10-19T00:00:00Z')
        for name, doc, numbered in cases():
            folder = out / profile / name
            folder.mkdir(parents=True)
            plan = save_source(doc, folder)
            data = doc.render_pdf(renderer, a11y_mode='raise')
            row = inspect_saved(data, folder, preview=profile == 'pdfua2',
                                blank_pages=(2,) if name.startswith('blank-') else ())
            errors = row['errors']
            if data != doc.render_pdf(renderer) or data != bytes(renderer.engine_for(doc).render_pdf(plan.html, plan.css)):
                errors.append('Component replay or direct HTML/CSS bytes differ')
            actual = texts(data)
            if len(actual) != 3:
                errors.append('Expected exactly three pages')
            for index, text in enumerate(actual, 1):
                if (f'Page {index} of 3' in text) != (index in numbered):
                    errors.append('Incorrect finalized page value or suppression')
            control = Document(*doc.children, title=doc.title, lang=doc.lang, theme=doc.theme,
                               page=replace(doc.page, header=None, footer=None))
            control_roles = INSPECT(control.render_pdf(renderer))['roles']
            if row['roles'] != control_roles:
                errors.append('Repeated band altered body structure roles')
            row.update(name=name, profile=profile, kind='document', ok=not errors)
            (folder / 'inspection.json').write_text(json.dumps(row, indent=2) + '\n', encoding='utf-8')
            report['cases'].append(row)

        people = ['Ada', 'Beatrice', 'Ada']
        messages = ['Short record.', 'A longer record with text that flows onto more pages. ' * 32, 'Short record.']
        controls = [record_document(person, message).render_pdf(renderer, a11y_mode='raise') for person, message in zip(people, messages)]
        control_texts = [texts(data) for data in controls]
        assert len(control_texts[0]) == 1 and len(control_texts[1]) > 1
        expected_roles = Counter()
        for control in controls:
            expected_roles.update({role: count for role, count in INSPECT(control)['roles'].items() if role != '/Document'})
        template = record_document('{{person}}', '{{message}}')
        prepared = template.prepare(renderer, a11y_mode='raise')
        for mode in ('throughput', 'compact'):
            folder = out / profile / ('records-' + mode)
            folder.mkdir(parents=True)
            save_source(template, folder)
            for index, control in enumerate(controls, 1):
                (folder / f'control-{index}.pdf').write_bytes(control)
            bindings = {'person': people, 'message': messages}
            data = bytes(prepared.render_pdf_reflow_bindings(bindings, compression=mode))
            row = inspect_saved(data, folder, preview=profile == 'pdfua2' and mode == 'throughput')
            errors = row['errors']
            if texts(data) != [page for record in control_texts for page in record]:
                errors.append('Record text or finalized page values differ from independent records')
            if row['pixels'] != [page for control in controls for page in PIXELS(control)]:
                errors.append('Record pixels differ from independent records')
            if {role: count for role, count in row['roles'].items() if role != '/Document'} != dict(expected_roles):
                errors.append('Record batch changed body structure roles')
            if data != bytes(prepared.render_pdf_reflow_bindings(bindings, compression=mode)):
                errors.append('Repeated record batch bytes differ')
            row.update(name='records-' + mode, profile=profile, kind='records', ok=not errors,
                       record_pages=[len(record) for record in control_texts])
            (folder / 'inspection.json').write_text(json.dumps(row, indent=2) + '\n', encoding='utf-8')
            report['cases'].append(row)

        for name, content, reason in (
            ('tall-component', Paragraph('Too tall', style=Style(height=pt(100))), 'measured_box_overflow'),
            ('translated-numbers', PageNumbers(style=css('transform:translateX(500pt)')), 'paint_overflow'),
        ):
            doc = document(footer=PageBand(right=content))
            folder = out / profile / name
            folder.mkdir(parents=True)
            save_source(doc, folder)
            try:
                doc.render_pdf(renderer)
            except PageBandError as error:
                payload = error.to_dict()
                ok = error.reason == reason and error.slot == 'bottom-right' and error.page == 1 and bool(error.sources)
                (folder / 'diagnostic.json').write_text(json.dumps(payload, indent=2) + '\n', encoding='utf-8')
            else:
                payload, ok = None, False
            report['cases'].append(dict(name=name, profile=profile, kind='rejection', payload=payload,
                                        ok=ok, errors=[] if ok else ['Missing expected source-aware rejection']))
    report['ok'] = all(row['ok'] for row in report['cases'])
    (out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    result = check(parser.parse_args().out)
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']),
                         failed=[dict(name=row['name'], profile=row['profile'], errors=row['errors'])
                                 for row in result['cases'] if not row['ok']])))
    raise SystemExit(0 if result['ok'] else 1)
