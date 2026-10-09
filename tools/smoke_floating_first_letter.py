#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check horizontal floated initials in installed PDF and native-preview output.

The retained Chrome print observations use the same explicit bundled Noto Sans
font. Bounds allow font-outline/raster rounding, not general pixel parity.
CSS model: https://www.w3.org/TR/CSS22/selector.html#first-letter
Requires the optional pypdfium2 and Pillow verification dependencies.
"""
from __future__ import annotations

import argparse
import hashlib
from importlib import metadata
import io
import json
from pathlib import Path

import fullbleed

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / 'python/fullbleed_assets/fonts/NotoSans-Regular.ttf'
EXPECTATIONS = ROOT / 'tests/fixtures/first-letter-floats/chrome.json'


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def inspect_pdf(data: bytes) -> tuple[list, list]:
    import pypdfium2
    document = pypdfium2.PdfDocument(data)
    pages, previews = [], []
    try:
        for page in document:
            text = page.get_textpage()
            try:
                chars = [[text.get_text_range(i, 1), *text.get_charbox(i)]
                         for i in range(text.count_chars()) if text.get_text_range(i, 1).strip()]
                pages.append({'size': list(page.get_size()), 'chars': chars})
                previews.append(page.render(scale=150 / 72).to_pil())
            finally:
                text.close()
                page.close()
    finally:
        document.close()
    return pages, previews


def color_bounds(image, color=(188, 108, 37)) -> list[int] | None:
    from PIL import Image, ImageFilter
    rgb = image.convert('RGB')
    # Only the brown initial, excluding green body text and red borders.
    mask = Image.new('1', rgb.size)
    pixels = rgb.tobytes()
    mask.putdata([int(abs(r - color[0]) < 24 and abs(g - color[1]) < 24 and abs(b - color[2]) < 24)
                  for r, g, b in zip(pixels[0::3], pixels[1::3], pixels[2::3])])
    if color == (255, 224, 128):
        # Ignore isolated background-colored antialias fringes at border
        # corners. Measure the solid fill interior; border bounds are checked
        # independently. The same one-pixel erosion is used for both renderers.
        mask = mask.convert('L').filter(ImageFilter.MinFilter(3))
    bounds = mask.getbbox()
    return list(bounds) if bounds else None


COLORS = {'initial': (188, 108, 37), 'border': (224, 64, 32), 'background': (255, 224, 128)}


def compare_colors(views, expected):
    results = []
    for index, (view, page) in enumerate(zip(views, expected), 1):
        for name, color in COLORS.items():
            bounds = color_bounds(view, color)
            bounds = [value * 72 / 150 for value in bounds] if bounds else None
            reference = page['colorBoundsPt'][name]
            delta = (max(abs(a-b) for a,b in zip(bounds, reference))
                     if bounds is not None and reference is not None
                     else 0 if bounds == reference else None)
            results.append({'page': index, 'color': name, 'boundsPt': bounds,
                            'deltaPt': delta, 'ok': delta is not None and delta <= 1.5})
    return results


def compare_pages(actual: list, expected: list, tolerance: float) -> dict:
    same_pages = len(actual) == len(expected)
    same_text = same_pages and all(
        ''.join(char[0] for char in a['chars']) == ''.join(char[0] for char in e['chars'])
        for a, e in zip(actual, expected))
    same_sizes = same_pages and all(a['size'] == e['size'] for a, e in zip(actual, expected))
    delta = max((abs(x - y) for a, e in zip(actual, expected)
                 for ac, ec in zip(a['chars'], e['chars'])
                 for x, y in zip(ac[1:], ec[1:])), default=0.0) if same_text else None
    return {'ok': same_text and same_sizes and delta is not None and delta <= tolerance,
            'sameText': same_text, 'samePageSizes': same_sizes,
            'maximumCharBoundsDeltaPt': delta, 'tolerancePt': tolerance}


def check(output: Path) -> dict:
    from PIL import Image
    output.mkdir(parents=True, exist_ok=True)
    oracle = json.loads(EXPECTATIONS.read_text(encoding='utf-8'))
    assert sha(FONT.read_bytes()) == oracle['fontSha256'], 'Refresh the explicit font/reference together'
    engine = fullbleed.PdfEngine(font_files=[str(FONT)])
    report = {'schema': 'fullbleed.floating_first_letter.v1', 'version': metadata.version('fullbleed'),
              'ok': False, 'reference': oracle['reference'], 'cases': [],
              'scope': 'Horizontal first-letter floats in plain-text blocks; not general CSS parity or accessibility conformance.'}
    for case in oracle['cases']:
        name, html, css = case['name'], case['html'], case['css']
        folder = output / name
        folder.mkdir(exist_ok=True)
        (folder / 'source.html').write_text(html, encoding='utf-8')
        (folder / 'source.css').write_text(css, encoding='utf-8')
        direct = bytes(engine.render_pdf(html, css))
        compiled = engine.compile_pdf(html, css)
        template = html.replace(case['text'], '{{text}}')
        reflow = engine.compile_pdf(template, css)
        documents = {'direct': direct, 'compiled': bytes(compiled.render_pdf()),
                     'reflow': bytes(reflow.render_pdf_reflow_bindings({'text': [case['text']]}))}
        result = {'name': name, 'pdfs': {}, 'native': [],
                  'deterministic': direct == bytes(engine.render_pdf(html, css)),
                  'compiledReplayStable': documents['compiled'] == bytes(compiled.render_pdf())}
        direct_views = []
        for mode, data in documents.items():
            (folder / f'{mode}.pdf').write_bytes(data)
            pages, views = inspect_pdf(data)
            result['pdfs'][mode] = {'sha256': sha(data), 'pages': len(pages),
                                   **compare_pages(pages, case['pages'], case['tolerancePt'])}
            color_checks = compare_colors(views, case['pages'])
            result['pdfs'][mode]['colors'] = color_checks
            result['pdfs'][mode]['ok'] &= all(check['ok'] for check in color_checks)
            if mode == 'direct':
                direct_views = views
                for index, view in enumerate(views, 1):
                    view.save(folder / f'pdfium-{index}.png')
        native = list(engine.render_image_pages(html, css, 150))
        result['sameNativePageCount'] = len(native) == len(direct_views)
        for index, (png, final) in enumerate(zip(native, direct_views), 1):
            data = bytes(png)
            (folder / f'native-{index}.png').write_bytes(data)
            with Image.open(io.BytesIO(data)) as view:
                native_bounds, pdf_bounds = color_bounds(view), color_bounds(final)
                if native_bounds is None or pdf_bounds is None:
                    delta = 0 if native_bounds == pdf_bounds else None
                else:
                    delta = max(abs(a - b) for a, b in zip(native_bounds, pdf_bounds)) * 72 / 150
                result['native'].append({'page': index, 'nativeInitialBoundsPx': native_bounds,
                                         'pdfInitialBoundsPx': pdf_bounds, 'deltaPt': delta,
                                         'colors': compare_colors([view], [case['pages'][index-1]]),
                                         'ok': view.size == final.size and delta is not None and delta <= 1.5})
                result['native'][-1]['ok'] &= all(check['ok'] for check in result['native'][-1]['colors'])
        result['ok'] = (result['deterministic'] and result['compiledReplayStable']
                        and result['sameNativePageCount']
                        and all(view['ok'] for view in result['native'])
                        and all(pdf['ok'] for pdf in result['pdfs'].values()))
        report['cases'].append(result)
    report['ok'] = all(case['ok'] for case in report['cases'])
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    report = check(args.out)
    print(json.dumps({'ok': report['ok'], 'cases': len(report['cases']),
                      'failed': [c['name'] for c in report['cases'] if not c['ok']]}, indent=2))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
