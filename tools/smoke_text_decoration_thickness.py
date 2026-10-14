#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify font-selected decoration thickness in installed PDFs and previews.

CSS reference: https://www.w3.org/TR/css-text-decor-4/#text-decoration-thickness-property
The expected stroke widths come from the explicit font's OpenType post table,
read independently with fontTools. Explicit-length controls must produce the
same pixels in each renderer; this is not a browser pixel-parity assertion.
Development-only dependencies: fonttools, pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fontTools.ttLib import TTFont
from PIL import Image
from pypdf import PdfReader
from pypdf.generic import ContentStream
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
FONTS = {
    'noto': ROOT / 'python/fullbleed_assets/fonts/NotoSans-Regular.ttf',
    'inter': ROOT / 'python/fullbleed_assets/fonts/Inter-Variable.ttf',
    'no-preference': ROOT / 'src/preview_fonts/FullbleedPreviewSans-Regular.ttf',
}
MODES = ('direct', 'compiled', 'fixed', 'reflow', 'compact')
HTML = '<p>Invoice {{label}} HHH</p>'
BASE = '''@page {size:480px 140px;margin:0}
html,body {margin:0;padding:0;color:black;background:white}
p {position:absolute;left:24px;top:30px;margin:0;line-height:64px;
text-decoration-color:#d00000;text-decoration-skip-ink:none;text-underline-offset:8px}
'''


def line_widths(page, reader):
    """Read colored stroke widths, including compiled PDF Form XObjects."""
    widths = []

    def visit(stream, resources, width=1.0, color=None, depth=0):
        assert depth < 12, 'unexpected recursive Form XObject'
        stack = []
        for args, op in ContentStream(stream, reader).operations:
            if op == b'q':
                stack.append((width, color))
            elif op == b'Q':
                width, color = stack.pop()
            elif op == b'w':
                width = float(args[0])
            elif op == b'RG':
                color = tuple(map(float, args))
            elif op in (b'S', b's') and color and color[0] > .7 and color[1:] == (0., 0.):
                widths.append(width)
            elif op == b'Do':
                obj = resources['/XObject'][args[0]].get_object()
                if obj.get('/Subtype') == '/Form':
                    visit(obj, obj.get('/Resources', resources), width, color, depth + 1)

    visit(page.get_contents(), page['/Resources'])
    return widths


def font_width(path, size_px):
    with TTFont(path) as font:
        preferred = font['post'].underlineThickness
        units = font['head'].unitsPerEm
    # The engine has millipoint geometry; round once after scaling the native
    # font units. CSS line decoration retains its existing one-CSS-pixel floor.
    value = round(size_px * .75 * preferred / units, 3) if preferred > 0 else int(size_px * .1) * .75
    return max(.75, value), {'units_per_em': units, 'underline_thickness': preferred}


def cases():
    for font, size, family in [('noto', 40, 'Probe'), ('inter', 40, 'Probe'),
                               ('noto', 12, 'Probe'), ('noto', 40, 'Missing,Probe'),
                               ('no-preference', 40, 'Probe')]:
        expected, metrics = font_width(FONTS[font], size)
        for lines in ['underline', 'overline', 'line-through', 'underline overline line-through']:
            name = f'{font}-{size}-{family.replace(",", "-")}-{lines.replace(" ", "-")}'
            yield dict(name=name, font=font, size=size, family=family,
                       lines=lines, expected=expected, metrics=metrics)


def pixels(png):
    with Image.open(BytesIO(png)) as img:
        rgb = img.convert('RGB')
        data = rgb.tobytes()
        assert has_red(data), 'missing red decoration'
        return sha256(data).hexdigest()


def has_red(data):
    return any(r > 150 and g < 60 and b < 60 for r, g, b in zip(data[0::3], data[1::3], data[2::3]))


def render(engine, css, mode):
    labels = ['AAA'] if mode in ('direct', 'compiled') else ['AAA', 'BBB']
    if mode == 'direct':
        action = lambda: engine.render_pdf(HTML.replace('{{label}}', labels[0]), css)
    elif mode == 'compiled':
        compiled = engine.compile_pdf(HTML.replace('{{label}}', labels[0]), css)
        action = compiled.render_pdf
    else:
        compiled = engine.compile_pdf(HTML, css)
        if mode == 'fixed':
            action = lambda: compiled.render_pdf_bindings({'label': labels})
        else:
            action = lambda: compiled.render_pdf_reflow_bindings(
                {'label': labels}, compression='compact' if mode == 'compact' else 'throughput')
    data = bytes(action())
    assert data == bytes(action()), f'{mode}: nondeterministic PDF'
    return data, labels


def check(out):
    out.mkdir(parents=True, exist_ok=True)
    (out / 'input-template.html').write_text(HTML, encoding='utf-8')
    report = {'schema': 'fullbleed.text_decoration_thickness.v1', 'ok': False,
              'versions': {name: metadata.version(name) for name in
                           ['fullbleed', 'fonttools', 'pypdf', 'pypdfium2', 'pillow']},
              'fonts': {name: sha256(path.read_bytes()).hexdigest() for name, path in FONTS.items()},
              'scope': __doc__, 'cases': [], 'failures': []}
    for case in cases():
        engine = fullbleed.PdfEngine()
        bundle = fullbleed.AssetBundle()
        bundle.add_file(str(FONTS[case['font']]), fullbleed.AssetKind.Font, name='Probe')
        engine.register_bundle(bundle)
        for mode in MODES:
            name = case['name'] + '-' + mode
            folder = out / name
            folder.mkdir(exist_ok=True)
            row = {'name': name, 'expected_pt': case['expected'], 'font_metrics': case['metrics'], 'variants': {}}
            for variant, thickness in [('font', 'from-font'), ('length', f'{case["expected"]}pt'), ('auto', 'auto')]:
                css = BASE + (f'p {{font-family:{case["family"]};font-size:{case["size"]}px;'
                              f'text-decoration-line:{case["lines"]};text-decoration-thickness:{thickness}}}')
                (folder / f'{variant}.css').write_text(css, encoding='utf-8')
                data, labels = render(engine, css, mode)
                path = folder / f'{variant}.pdf'
                path.write_bytes(data)
                reader = PdfReader(BytesIO(data))
                assert len(reader.pages) == len(labels), name
                assert [' '.join(p.extract_text().split()) for p in reader.pages] == [
                    f'Invoice {label} HHH' for label in labels], name
                widths = [line_widths(page, reader) for page in reader.pages]
                assert all(len(w) == len(case['lines'].split()) for w in widths), (name, widths)
                hashes = {'pdfium': [], 'finalized': []}
                with pdfium.PdfDocument(data) as doc:
                    for i, page in enumerate(doc):
                        bitmap = page.render(scale=2)
                        img = bitmap.to_pil().convert('RGB')
                        assert has_red(img.tobytes()), name
                        hashes['pdfium'].append(sha256(img.tobytes()).hexdigest())
                        img.save(folder / f'{variant}-pdfium-{i+1}.png')
                        img.close(); bitmap.close(); page.close()
                for i, png in enumerate(engine.render_finalized_pdf_image_pages(str(path), 144)):
                    png = bytes(png)
                    hashes['finalized'].append(pixels(png))
                    (folder / f'{variant}-finalized-{i+1}.png').write_bytes(png)
                if mode == 'direct':
                    png, = engine.render_image_pages(HTML.replace('{{label}}', 'AAA'), css, 144)
                    hashes['native'] = [pixels(bytes(png))]
                    (folder / f'{variant}-native.png').write_bytes(bytes(png))
                assert len(hashes['pdfium']) == len(hashes['finalized']) == len(labels), name
                row['variants'][variant] = {'widths_pt': widths, 'pixels_sha256': hashes,
                                            'pdf_sha256': sha256(data).hexdigest()}
            font = row['variants']['font']
            length = row['variants']['length']
            auto = row['variants']['auto']
            row['preferred_width'] = all(abs(width-case['expected']) <= .0005
                                        for page in font['widths_pt'] for width in page)
            row['matches_explicit_pixels'] = font['pixels_sha256'] == length['pixels_sha256']
            should_differ = case['font'] != 'no-preference' and case['size'] == 40
            row['auto_control'] = (font['pixels_sha256'] != auto['pixels_sha256']) == should_differ
            row['ok'] = row['preferred_width'] and row['matches_explicit_pixels'] and row['auto_control']
            if not row['ok']:
                report['failures'].append(name)
            report['cases'].append(row)
    report['ok'] = not report['failures']
    (out / 'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({'ok': result['ok'], 'cases': len(result['cases']), 'failures': result['failures']}))
    raise SystemExit(0 if result['ok'] else 1)
