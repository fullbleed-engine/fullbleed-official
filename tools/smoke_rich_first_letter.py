#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check styled first-letter output against retained independent print geometry.

Uses the optional PDFium and Pillow test dependencies. This is a focused
regression gate, not a general CSS or accessibility conformance claim.
"""
from __future__ import annotations

import argparse
from importlib import metadata
import io
import json
from pathlib import Path
import runpy

import fullbleed

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / 'python/fullbleed_assets/fonts/NotoSans-Regular.ttf'
REFERENCE = ROOT / 'tests/fixtures/rich-first-letter/chrome.json'


def compare_geometry(actual: list, expected: list, tolerance: float) -> dict:
    same_pages = len(actual) == len(expected)
    same_text = same_pages and all(
        ''.join(char[0] for char in a['chars']) == ''.join(char[0] for char in e['chars'])
        for a, e in zip(actual, expected))
    # Chromium rounds some physical sheet sizes to its print-device grid.
    # Compare top-based glyph bounds without importing that sheet rounding.
    same_sizes = same_pages and all(
        max(abs(x-y) for x, y in zip(a['size'], e['size'])) <= 0.25
        for a, e in zip(actual, expected))
    delta = max((max(abs(ac[1]-ec[1]), abs(ac[3]-ec[3]),
                     abs((a['size'][1]-ac[2])-(e['size'][1]-ec[2])),
                     abs((a['size'][1]-ac[4])-(e['size'][1]-ec[4])))
                 for a, e in zip(actual, expected)
                 for ac, ec in zip(a['chars'], e['chars'])), default=0) if same_text else None
    return {'ok': same_text and same_sizes and delta is not None and delta <= tolerance,
            'sameText': same_text, 'samePageSizes': same_sizes,
            'maximumCharBoundsDeltaPt': delta, 'tolerancePt': tolerance}


def check(output: Path) -> dict:
    from PIL import Image
    helpers = runpy.run_path(str(ROOT / 'tools/smoke_floating_first_letter.py'))
    sha, inspect = helpers['sha'], helpers['inspect_pdf']
    compare_colors = helpers['compare_colors']
    output.mkdir(parents=True, exist_ok=True)
    oracle = json.loads(REFERENCE.read_text(encoding='utf-8'))
    assert sha(FONT.read_bytes()) == oracle['fontSha256']
    engine = fullbleed.PdfEngine(font_files=[str(FONT)])
    report = {'schema': 'fullbleed.rich_first_letter.v1', 'version': metadata.version('fullbleed'),
              'reference': oracle['reference'], 'scope': oracle['scope'], 'cases': [], 'ok': False}
    for case in oracle['cases']:
        folder = output / case['name']
        folder.mkdir(exist_ok=True)
        html, css = case['html'], case['css']
        (folder / 'source.html').write_text(html, encoding='utf-8')
        (folder / 'source.css').write_text(css, encoding='utf-8')
        direct = bytes(engine.render_pdf(html, css))
        compiled = engine.compile_pdf(html, css)
        token = 'Documents' if 'Documents' in html else 'D'
        reflow = engine.compile_pdf(html.replace(token, '{{opening}}', 1), css)
        documents = {'direct': direct, 'compiled': bytes(compiled.render_pdf()),
                     'reflow': bytes(reflow.render_pdf_reflow_bindings({'opening': [token]}))}
        row = {'name': case['name'], 'pdfs': {}, 'native': [],
               'deterministic': direct == bytes(engine.render_pdf(html, css)),
               'compiledReplayStable': documents['compiled'] == bytes(compiled.render_pdf())}
        trace = engine.export_render_time_reading_order_trace(html, css)
        if isinstance(trace, str):
            trace = json.loads(trace)
        (folder / 'reading-order.json').write_text(json.dumps(trace, indent=2) + '\n', encoding='utf-8')
        text_blocks = [block for page in trace['pages'] for block in page['blocks']
                       if block.get('text', '').strip()]
        reading_text = ''.join(''.join(block['text'].split()) for block in text_blocks)
        expected_text = ''.join(char[0] for page in case['pages'] for char in page['chars'])
        paragraph_tags = all('P' in block.get('tag_path', []) for block in text_blocks)
        row['readingOrder'] = {'sameText': reading_text == expected_text,
                               'paragraphTagOwnership': paragraph_tags,
                               'ok': trace['ok'] and reading_text == expected_text and paragraph_tags}
        direct_pages = None
        for mode, data in documents.items():
            (folder / f'{mode}.pdf').write_bytes(data)
            pages, views = inspect(data)
            colors = compare_colors(views, case['pages'])
            checks = compare_geometry(pages, case['pages'], case['tolerancePt'])
            if direct_pages is None:
                direct_pages = pages
                for index, view in enumerate(views, 1):
                    view.save(folder / f'pdfium-{index}.png')
            replay = compare_geometry(pages, direct_pages, 0.0)
            row['pdfs'][mode] = {**checks, 'sha256': sha(data), 'pages': len(pages),
                                 'colors': colors, 'matchesDirectGeometry': replay['ok']}
            row['pdfs'][mode]['ok'] &= all(c['ok'] for c in colors) and replay['ok']
        native = list(engine.render_image_pages(html, css, 150))
        row['sameNativePageCount'] = len(native) == len(case['pages'])
        for index, (png, page) in enumerate(zip(native, case['pages']), 1):
            data = bytes(png)
            (folder / f'native-{index}.png').write_bytes(data)
            with Image.open(io.BytesIO(data)) as view:
                colors = compare_colors([view], [page])
                row['native'].append({'page': index, 'colors': colors,
                                      'ok': all(c['ok'] for c in colors)})
        row['ok'] = (row['deterministic'] and row['compiledReplayStable'] and row['sameNativePageCount']
                     and row['readingOrder']['ok']
                     and all(pdf['ok'] for pdf in row['pdfs'].values())
                     and all(view['ok'] for view in row['native']))
        report['cases'].append(row)
    report['ok'] = all(case['ok'] for case in report['cases'])
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    report = check(args.out)
    print(json.dumps({'ok': report['ok'], 'cases': len(report['cases']),
                      'failed': [case['name'] for case in report['cases'] if not case['ok']]}))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
