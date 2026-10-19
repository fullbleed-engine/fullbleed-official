#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Saved-PDF checks for the internal managed page-margin paint gate.

The expanded-BBox files are explicitly altered inspection aids. Comparing them
with engine output detects clipping in these fixtures; raster sampling is not
the implementation's general overflow test or a standards-conformance test.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata
import io
import json
from pathlib import Path

import fullbleed
from PIL import ImageChops
from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, FloatObject, NameObject
import pypdfium2

BASE = '''@page {size:320pt 240pt;margin:80pt 20pt 40pt;
@top-left {content:element(head);width:100pt}}
* {margin:0;box-sizing:border-box}
html,body {font:10pt/12pt "Fullbleed Preview Mono"}
.band {position:running(head);background:#d9ede3;height:30pt}
.child {background:#244c85;color:white;height:20pt;width:80pt}
'''


def source(text='VISIBLE'):
    return ('<!doctype html><html lang="en-US"><head><title>Managed margin paint</title></head><body>'
            '<div class="band" data-fb-running-layout="band" data-fb-id="band"><div class="child">'
            + text + '</div></div><main><h1>Body control</h1></main></body></html>')


def pixels(data):
    with pypdfium2.PdfDocument(data) as pdf:
        assert len(pdf) == 1
        page = pdf[0]
        bitmap = page.render(scale=2)
        result = bitmap.to_pil().convert('RGB')
        bitmap.close()
        page.close()
        return result


def expanded_bbox(data):
    writer = PdfWriter()
    writer.clone_document_from_reader(PdfReader(io.BytesIO(data)))
    changed = []
    available = []
    for name, ref in writer.pages[0]['/Resources'].get('/XObject', {}).items():
        obj = ref.get_object()
        bounds = list(map(float, obj['/BBox'])) if obj.get('/Subtype') == '/Form' else None
        if bounds is not None:
            available.append(dict(name=str(name), bounds=bounds))
        # Collapsed child margins can change measured height. Select the
        # fixture's allocated 100pt slot, without assuming the CSS height is
        # the complete measured Form height.
        if bounds is not None and bounds[:3] == [0, 0, 100] and bounds[3] > 0:
            changed.append(str(name))
            obj[NameObject('/BBox')] = ArrayObject([FloatObject(v) for v in (-1000, -1000, 1000, 1000)])
    assert len(changed) == 1, dict(expected='one 100pt running Form', changed=changed, forms=available)
    buffer = io.BytesIO()
    writer.write(buffer)
    return buffer.getvalue()


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    core = Path(__file__).resolve().parents[1]
    fonts = [core/'src/preview_fonts'/('FullbleedPreview'+name+'-Regular.ttf') for name in ('Mono', 'Sans')]
    fonts.append(core/'tests/fixtures/rtl/NotoSansArabic-Regular.ttf')
    report = dict(schema='fullbleed.page_margin_paint_smoke.v1', ok=False, cases=[],
                  versions={name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2')},
                  fonts={p.name: sha256(p.read_bytes()).hexdigest() for p in fonts},
                  scope='Fixture paint-fit, deterministic output and independently rendered clipping checks; no conformance claim.')
    valid = [
        ('control', '', 'VISIBLE'),
        ('explicit-clip', '.band {overflow:hidden} .child {width:150pt}', 'VISIBLE'),
        ('descenders', '', 'gypj'),
        ('shaping', '.child {font-family:"Fullbleed Preview Sans"}', 'ffi A\u0301'),
        ('rtl', '.child {font-family:"Noto Sans Arabic";direction:rtl}', 'مرحبا'),
        ('synthetic-bold', '.child {font-weight:700}', 'Bold gyp'),
        ('synthetic-italic', '.child {font-style:italic}', 'Italic'),
        ('contained-rotation', '.child {transform:rotate(5deg);width:60pt;height:12pt;margin:8pt}', 'TURN'),
        ('negative-scale', '.child {transform:scaleX(-1);width:60pt;margin-left:10pt}', 'MIRROR'),
        ('nested-svg', '', '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="12" viewBox="0 0 40 12"><path d="M2 2H38V10H2Z" fill="#ffe100"/></svg>'),
    ]
    invalid = [
        ('wide-child', '.child {width:150pt}', 'paint_overflow', None),
        ('tall-child', '.child {height:70pt}', 'paint_overflow', None),
        ('relative-child', '.child {position:relative;left:65pt}', 'paint_overflow', None),
        ('absolute-child', '.child {position:absolute;left:65pt}', 'paint_overflow', None),
        ('translated-child', '.child {transform:translateX(65pt)}', 'paint_overflow', None),
        ('rotated-child', '.child {transform:rotate(30deg)}', 'paint_overflow', None),
        ('shadow-child', '.child {box-shadow:25pt 0 5pt #f00000}', 'paint_bounds_unknown', 'filtered_form_extent'),
        ('zero-height-positioned', '.band {height:0}.child {position:absolute;top:0;left:0}', 'paint_overflow', None),
        ('outside-reserved-margin', '@page {@top-left {width:400pt}}', 'slot_outside_reserved_margin', None),
    ]
    for profile in ('tagged', 'pdfua1', 'pdfua2'):
        engine = fullbleed.PdfEngine(font_files=list(map(str, fonts)), pdf_profile=profile,
            document_title='Managed margin paint', document_lang='en-US', document_timestamp='2026-10-19T00:00:00Z')
        for name, extra, text in valid:
            folder = out/profile/name
            folder.mkdir(parents=True)
            html, css = source(text), BASE+extra
            (folder/'source.html').write_text(html, encoding='utf-8')
            (folder/'source.css').write_text(css, encoding='utf-8')
            errors = []
            row = dict(name=name, profile=profile, expected='accepted', errors=errors)
            try:
                data = bytes(engine.render_pdf(html, css))
                legacy = bytes(engine.render_pdf(html.replace('layout="band"', 'layout="margin"'), css))
                if data != bytes(engine.render_pdf(html, css)): errors.append('Repeated output changed')
                (folder/'engine.pdf').write_bytes(data)
                altered = expanded_bbox(data)
                (folder/'inspection-expanded-bbox.pdf').write_bytes(altered)
                original, expanded = pixels(data), pixels(altered)
                if ImageChops.difference(original, pixels(legacy)).getbbox():
                    errors.append('Managed opt-in changed the accepted fixture pixels')
                delta = ImageChops.difference(original, expanded)
                changed = sum(pixel != (0,0,0) for pixel in delta.getdata())
                original.save(folder/'engine.png')
                if changed: errors.append('Accepted fixture hides paint behind outer Form BBox')
                row.update(pdf_sha256=sha256(data).hexdigest(), changed_pixels=changed)
            except (ValueError, AssertionError) as error:
                errors.append(str(error))
            row['ok'] = not errors
            report['cases'].append(row)
        for name, extra, reason, detail in invalid:
            folder = out/profile/name
            folder.mkdir(parents=True)
            html, css = source(), BASE+extra
            (folder/'source.html').write_text(html, encoding='utf-8')
            (folder/'source.css').write_text(css, encoding='utf-8')
            errors, payload = [], None
            try:
                engine.render_pdf(html, css)
                errors.append('Expected paint-fit rejection')
            except ValueError as error:
                payload = json.loads(str(error).split('page-margin layout: ',1)[1])
                for key, value in dict(reason=reason, page=1, slot='top-left', source_ids=['band'], units='millipoints').items():
                    if payload.get(key) != value: errors.append('Unexpected diagnostic field: '+key)
                if detail and payload.get('paint_check',{}).get('reason') != detail:
                    errors.append('Missing actionable unknown-paint reason')
            report['cases'].append(dict(name=name, profile=profile, expected='rejected', payload=payload, ok=not errors, errors=errors))
    report['ok'] = all(row['ok'] for row in report['cases'])
    (out/'verification.json').write_text(json.dumps(report, indent=2, ensure_ascii=False)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    result = check(parser.parse_args().out)
    print(json.dumps(dict(ok=result['ok'], cases=len(result['cases']), failures=[row for row in result['cases'] if not row['ok']]), ensure_ascii=False))
    raise SystemExit(0 if result['ok'] else 1)
