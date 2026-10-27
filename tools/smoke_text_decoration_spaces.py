#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check decorations across interior spaces in installed PDFs and previews.

CSS reference: https://www.w3.org/TR/css-text-decor-3/#line-decoration
Independently rasterize saved PDFs and require continuous solid colored lines
across word spaces. Check collapsed line edges, authored decoration boundaries,
atomic inlines, propagated baselines, shadows, and unchanged glyph positions.
This is a scoped rendering regression check, not a browser-parity claim.
Development-only dependencies: pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from PIL import Image
from pypdf import PdfReader
import pypdfium2 as pdfium

MODES = ('direct', 'compiled', 'fixed', 'reflow', 'compact')
BASE = '''@page {size:480px 260px;margin:20px}
html,body {margin:0;padding:0;color:black;background:white}
body {font-family:Probe;font-size:24px;line-height:40px}
p {margin:0} .label {margin-top:40px;font-size:12px;color:#333}
.dec {text-decoration-line:underline;text-decoration-color:#d00000;
text-decoration-thickness:2px;text-underline-offset:3px;text-decoration-skip-ink:none}
'''


def cases():
    standard = '<p>HH <span class="dec">HH HH</span> HH</p>'
    for line in ('underline', 'overline', 'line-through', 'underline overline line-through'):
        yield dict(name=line.replace(' ', '-'), html=standard,
                   css=f'.dec {{text-decoration-line:{line}}}', bands=len(line.split()), intervals=1)
    for name, content, css, bands, intervals in (
        ('word-letter-spacing', standard, '.dec {word-spacing:12px;letter-spacing:3px}', 1, 1),
        ('nested-strong', '<p><span class="dec">HH <strong>HH HH</strong> HH</span></p>', '', 1, 1),
        ('propagated-raised', '<p><span class="dec">HH <span style="vertical-align:8px">HH HH</span> HH</span></p>', '', 1, 1),
        ('descendant-none', '<p><span class="dec">HH <span style="text-decoration:none">HH HH</span> HH</span></p>', '', 1, 1),
        ('wrapped', '<p><span class="dec"> HH HH HH HH </span></p>', 'p:not(.label) {width:100px}', 2, 1),
        ('nowrap-control', standard, '.dec {white-space:nowrap}', 1, 1),
        ('atomic-isolation', '<p><span class="dec">HH <span class="atom">KK KK</span> HH</span></p>', '.atom {display:inline-block;color:#0000b0}', 1, 2),
        ('authored-boundaries', '<p><span class="dec">HH</span> <span class="dec">HH</span></p>', '', 1, 2),
        ('collapsed-edges', '<p><span class="dec">   HH HH   </span></p>', '', 1, 1),
        ('collapsed-multiple', '<p><span class="dec">HH \n\t HH</span></p>', '', 1, 1),
        ('only-collapsed', '<p><span class="dec">  </span>HH</p>', '', 0, 0),
        ('decoration-shadow', standard, '.dec {text-shadow:0 8px 0 #0000b0}', 1, 1),
    ):
        case = dict(name=name, html=content, css=css, bands=bands, intervals=intervals)
        if name in ('collapsed-edges', 'collapsed-multiple'):
            case['control'] = '<p><span class="dec">HH HH</span></p>'
        yield case


def groups(values):
    result = []
    for value in sorted(set(values)):
        if not result or value > result[-1][-1] + 1:
            result.append([value])
        else:
            result[-1].append(value)
    return result


def check_image(image, case):
    rgb = image.convert('RGB')
    red = {}
    data = rgb.tobytes()
    stride = rgb.width * 3
    for y in range(rgb.height):
        row = data[y * stride:(y + 1) * stride]
        xs = [offset // 3 for offset in range(0, stride, 3)
              if row[offset] > 140 and row[offset + 1] < 80 and row[offset + 2] < 80]
        if xs:
            red[y] = xs
    bands = groups(red)
    assert len(bands) == case['bands'], (case['name'], 'decoration bands', len(bands), case['bands'])
    bounds = []
    for band in bands:
        intervals = groups(x for y in band for x in red[y])
        assert len(intervals) == case['intervals'], (case['name'], 'disconnected intervals', [(v[0], v[-1]) for v in intervals])
        bounds.append([intervals[0][0], band[0], intervals[-1][-1], band[-1]])
        if case['name'] == 'decoration-shadow':
            # CSS px -> pixels at 144 dpi. The shadow must bridge the same space.
            y = (band[0] + band[-1]) // 2 + 12
            for x in range(intervals[0][0] + 1, intervals[-1][-1]):
                r, g, b = rgb.getpixel((x, y))
                assert b > 140 and r < 80 and g < 80, ('shadow gap', x, y, (r, g, b))
    return {'bounds': bounds, 'pixels_sha256': sha256(rgb.tobytes()).hexdigest()}


def render(engine, html, css, mode):
    labels = ['AAA'] if mode in ('direct', 'compiled') else ['AAA', 'BBB']
    if mode == 'direct':
        action = lambda: engine.render_pdf(html.replace('{{record}}', labels[0]), css)
    elif mode == 'compiled':
        action = engine.compile_pdf(html.replace('{{record}}', labels[0]), css).render_pdf
    else:
        compiled = engine.compile_pdf(html, css)
        if mode == 'fixed':
            action = lambda: compiled.render_pdf_bindings({'record': labels})
        else:
            action = lambda: compiled.render_pdf_reflow_bindings(
                {'record': labels}, compression='compact' if mode == 'compact' else 'throughput')
    data = bytes(action())
    assert data == bytes(action()), f'{mode}: nondeterministic PDF'
    return data, labels


def glyph_geometry(data):
    result = []
    with pdfium.PdfDocument(data) as document:
        for page in document:
            text = page.get_textpage()
            result.append([(text.get_text_range(i, 1), tuple(round(v, 4) for v in text.get_charbox(i)))
                           for i in range(text.count_chars())])
            text.close()
            page.close()
    return result


def check(out, modes):
    out.mkdir(parents=True, exist_ok=True)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    engine = fullbleed.PdfEngine(document_timestamp='2026-10-20T00:00:00Z')
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(font), fullbleed.AssetKind.Font, name='Probe')
    engine.register_bundle(bundle)
    report = {'schema': 'fullbleed.text_decoration_spaces.v1', 'ok': False, 'scope': __doc__,
              'versions': {name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2', 'pillow')},
              'font_sha256': sha256(font.read_bytes()).hexdigest(), 'cases': [], 'failures': []}
    for case in cases():
        html = '<!doctype html><html lang="en"><head><title>Decoration spaces</title></head><body>' + case['html'] + '<p class="label">Record {{record}}</p></body></html>'
        css = BASE + case['css']
        for mode in modes:
            name = case['name'] + '-' + mode
            folder = out / name
            folder.mkdir(exist_ok=True)
            (folder / 'source.html').write_text(html, encoding='utf-8')
            (folder / 'source.css').write_text(css, encoding='utf-8')
            row = {'name': name, 'ok': False}
            try:
                data, labels = render(engine, html, css, mode)
                path = folder / 'output.pdf'
                path.write_bytes(data)
                row['pdf_sha256'] = sha256(data).hexdigest()
                reader = PdfReader(BytesIO(data))
                assert len(reader.pages) == len(labels)
                assert all(f'Record {label}' in page.extract_text() for label, page in zip(labels, reader.pages))
                undecorated, _ = render(engine, html, css + '.dec {text-decoration:none}', mode)
                assert glyph_geometry(data) == glyph_geometry(undecorated), 'decoration changed text or glyph positions'
                row['glyph_geometry_unchanged'] = True
                row['images'] = {}
                with pdfium.PdfDocument(data) as document:
                    for i, page in enumerate(document):
                        bitmap = page.render(scale=2)
                        image = bitmap.to_pil()
                        image.save(folder / f'pdfium-{i+1}.png')
                        row['images'][f'pdfium-{i+1}'] = check_image(image, case)
                        image.close(); bitmap.close(); page.close()
                for i, png in enumerate(engine.render_finalized_pdf_image_pages(str(path), 144)):
                    (folder / f'finalized-{i+1}.png').write_bytes(bytes(png))
                    with Image.open(BytesIO(bytes(png))) as image:
                        row['images'][f'finalized-{i+1}'] = check_image(image, case)
                if mode == 'direct':
                    png, = engine.render_image_pages(html.replace('{{record}}', 'AAA'), css, 144)
                    (folder / 'native.png').write_bytes(bytes(png))
                    with Image.open(BytesIO(bytes(png))) as image:
                        row['images']['native'] = check_image(image, case)
                if 'control' in case:
                    control_html = html.replace(case['html'], case['control'])
                    control, _ = render(engine, control_html, css, mode)
                    (folder / 'control.pdf').write_bytes(control)
                    with pdfium.PdfDocument(control) as document:
                        for i, page in enumerate(document):
                            bitmap = page.render(scale=2)
                            image = bitmap.to_pil().convert('RGB')
                            assert sha256(image.tobytes()).hexdigest() == row['images'][f'pdfium-{i+1}']['pixels_sha256'], 'collapsed whitespace changed paint'
                            image.close(); bitmap.close(); page.close()
                row['ok'] = True
            except Exception as error:
                row['error'] = str(error)
                report['failures'].append({'name': name, 'error': str(error)})
            report['cases'].append(row)
    report['ok'] = not report['failures']
    (out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--modes', nargs='+', choices=MODES, default=MODES)
    args = parser.parse_args()
    result = check(args.out, args.modes)
    print(json.dumps({'ok': result['ok'], 'cases': len(result['cases']), 'failures': result['failures']}), flush=True)
    raise SystemExit(0 if result['ok'] else 1)
