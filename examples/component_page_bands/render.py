#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Render the unpublished PageBand example; no dependencies beyond Fullbleed."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
import json
from pathlib import Path
import shutil

import fullbleed
from fullbleed.ui.experimental import Renderer
from report import build_document


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=Path(__file__).with_name('output'))
    parser.add_argument('--pdf-profile', choices=('tagged', 'pdfua1', 'pdfua2'), default='pdfua2')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    font = Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')))
    document = build_document()
    renderer = Renderer(font_files=[str(font)], pdf_profile=args.pdf_profile,
                        document_timestamp='2026-10-19T00:00:00Z')
    source = document.compile(a11y_mode='raise', target='pdf')
    prepared = document.prepare(renderer, a11y_mode='raise')
    data = prepared.render_pdf()
    assert data == document.render_pdf(renderer, a11y_mode='raise'), 'Replay must be deterministic'
    assert data == bytes(renderer.engine_for(document).render_pdf(source.html, source.css)), 'HTML/CSS replay differs'
    (out / 'field-notes.pdf').write_bytes(data)
    (out / 'source.html').write_text(source.html, encoding='utf-8', newline='\n')
    (out / 'source.css').write_text(source.css, encoding='utf-8', newline='\n')
    for styled, name in ((True, 'field-notes.html'), (False, 'field-notes-unstyled.html')):
        html = document.to_html(include_styles=styled, a11y_mode='raise')
        if styled:
            # Font loading and screen framing are export concerns, outside the
            # shared report components. No browser pagination is implied.
            html = html.replace('</head>', '''<style>
@font-face {font-family:Inter;src:url("fonts/Inter-Variable.ttf");font-weight:100 900}
body {max-width:540px;margin:0 auto;padding:24px}
[data-fb-page-band] {margin:12px 0 28px}
footer[data-fb-page-band] {margin-top:28px}
@media screen {
  [data-fb-flow] {display:flex;flex-direction:column}
  main > [data-fb-flow] {margin-bottom:40px}
}
</style></head>''', 1)
        (out / name).write_text(html, encoding='utf-8', newline='\n')
    (out / 'fonts').mkdir(exist_ok=True)
    shutil.copyfile(font, out / 'fonts' / font.name)
    inspection = fullbleed.inspect_pdf(str(out / 'field-notes.pdf'))
    report = dict(schema='fullbleed.component_page_bands_example.v1',
                  runtime=metadata.version('fullbleed'), pdf_profile=args.pdf_profile,
                  pdf_sha256=sha256(data).hexdigest(), font_sha256=sha256(font.read_bytes()).hexdigest(),
                  page_count=inspection['page_count'], deterministic=True, html_css_byte_parity=True,
                  external_validation='Run saved-PDF inspection and veraPDF separately; see README.md.')
    assert report['page_count'] == 3, report
    (out / 'render-report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    (out / 'a11y-source.json').write_text(json.dumps(source.a11y_report, indent=2) + '\n', encoding='utf-8')
    (out / 'pagination.json').write_text(json.dumps(prepared.export_render_time_pagination_trace(), indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
