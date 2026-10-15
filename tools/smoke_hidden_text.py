#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify hidden HTML text and author display overrides in installed PDF output.

HTML reference: https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements
Each input is compared with an independently authored visible-content control.
PDF bytes, extracted text, native previews, and independent PDFium pixels must
agree. No JavaScript is evaluated and embedded stylesheets remain unsupported.
Development-only dependencies: pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from html import escape
from importlib import metadata
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from PIL import Image
from pypdf import PdfReader
import pypdfium2 as pdfium

BASE = """@page {size:500px 300px;margin:20px}
html,body {margin:0;padding:0;color:black;background:white}
body {font-family:Helvetica;font-size:16px;line-height:24px}
h1 {font-size:18px;line-height:26px;margin:0 0 12px}
p,pre {margin:0} table {border-collapse:collapse} td {padding:0}
.flex {display:flex;gap:12px} .grid {display:grid;grid-template-columns:1fr 1fr}
.contents {display:contents} .decorated {color:#173b6b}
"""
MODES = ('direct', 'compiled', 'fixed', 'reflow')
SOURCE = 'SOURCE_SENTINEL'
CONTEXTS = {
    'blocks': '<p>Before</p>{source}<p>After</p>',
    'paragraph': '<p>Before {source} After</p>',
    'styled-inline': '<p>Before <span class="decorated">{source} After</span></p>',
    'contents': '<div class="contents"><p>Before</p>{source}<p>After</p></div>',
    'flex': '<div class="flex"><p>Before</p>{source}<p>After</p></div>',
    'grid': '<div class="grid"><p>Before</p>{source}<p>After</p></div>',
    'table-cell': '<table><tr><td>Before {source} After</td></tr></table>',
    'empty-table-cell': '<p>Before</p><table><tr><td>{source}</td></tr></table><p>After</p>',
    'caption': '<table><caption>Before {source} After</caption><tr><td>Cell</td></tr></table>',
    'css-caption': '<div style="display:table"><div style="display:table-caption">Before {source} After</div><div style="display:table-row"><div style="display:table-cell">Cell</div></div></div>',
    'list': '<ul style="list-style-position:inside"><li>Before {source} After</li></ul>',
    'pre': '<pre>Before {source} After</pre>',
}


def cases():
    def pair(name, context, source, control='', css='', kind='hidden'):
        return dict(name=name, html=context.format(source=source),
                    control=context.format(source=control), css=css, kind=kind)

    for tag in ('script', 'style'):
        source = f'<{tag} class="probe">{SOURCE}</{tag}>'
        for name, context in CONTEXTS.items():
            yield pair(f'{tag}-{name}', context, source)
        for display in ('inline', 'block', 'contents', 'initial', 'unset'):
            for name in ('paragraph', 'pre', 'caption', 'list'):
                yield pair(f'{tag}-{display}-{name}', CONTEXTS[name], source,
                           f'<fb-source class="probe">{SOURCE}</fb-source>',
                           f'.probe {{display:{display}}}', 'visible')
        yield pair(f'{tag}-inline-attribute', CONTEXTS['paragraph'],
                   f'<{tag} style="display:inline">{SOURCE}</{tag}>',
                   f'<fb-source style="display:inline">{SOURCE}</fb-source>', kind='visible')
        yield pair(f'{tag}-revert', CONTEXTS['blocks'], source,
                   css='.probe {display:block;display:revert}')
        yield pair(f'{tag}-hidden-parent', '<p>Before <span style="display:none">{source}</span> After</p>',
                   source, css='.probe {display:inline}')
        yield pair(f'{tag}-hidden-decoration', CONTEXTS['blocks'], source,
                   css='.probe {padding:30px;border:4px solid red;margin:20px} .probe::before {content:"UNEXPECTED"}')
        for name, context in [
            ('leading-space', '<div class="decorated">{source} After</div>'),
            ('trailing-space', '<div class="decorated">Before {source}</div>'),
            ('pseudo-space', '<p><span class="decorated">{source} After </span></p>'),
        ]:
            yield pair(f'{tag}-{name}', context, source,
                       f'<fb-source class="probe">{SOURCE}</fb-source>',
                       '.probe {display:inline;color:#751b38} .decorated::before {content:"Prefix"} .decorated::after {content:"Suffix"}', 'visible')
        raw = 'if (1 < 2) & "quoted";'
        yield pair(f'{tag}-literal-source', CONTEXTS['paragraph'],
                   f'<{tag} class="probe">{raw}</{tag}>',
                   f'<fb-source class="probe">{escape(raw)}</fb-source>',
                   '.probe {display:inline}', 'visible')

    for name in ('paragraph', 'pre', 'caption', 'list', 'css-caption', 'table-cell', 'empty-table-cell'):
        yield pair(f'hidden-span-{name}', CONTEXTS[name],
                   '<span style="display:none">HIDDEN_SENTINEL<br>LEAK</span>')
    yield pair('descendant-selector', '<p class="owner">Before {source} After</p>',
               '<span><span class="probe">HIDDEN_SENTINEL</span></span>',
               css='p.owner > span > .probe {display:none}')
    yield pair('hidden-break', '<p>Before{source}After</p>', '<br style="display:none">')
    yield pair('json-script', CONTEXTS['blocks'],
               '<script type="application/json">{"key":"HIDDEN_SENTINEL"}</script>')
    yield pair('external-script', CONTEXTS['blocks'],
               '<script src="https://example.invalid/never.js">HIDDEN_SENTINEL</script>')
    escaped = '<pre>&lt;script&gt;VISIBLE_LITERAL&lt;/script&gt;</pre>'
    yield dict(name='escaped-literal-control', html=escaped, control=escaped, css='', kind='control')
    fragment = CONTEXTS['blocks'].format(source='<script>HIDDEN_SENTINEL</script>')
    control = CONTEXTS['blocks'].format(source='')
    for name, head in [('full-document', '<title>Visibility probe</title>'),
                       ('head-source', '<title>Visibility probe</title><script>HEAD_SENTINEL</script><style>HEAD_STYLE</style>')]:
        yield dict(name=name, html=f'<html><head>{head}</head><body>{fragment}</body></html>',
                   control=f'<html><head>{head}</head><body>{control}</body></html>', css='', kind='hidden')


def render(engine, html, css, mode):
    # Full documents exercise the parser's body selection; all other cases also
    # exercise fixed/reflow bindings in visible content around the hidden nodes.
    if html.startswith('<html>'):
        html = html.replace('<body>', '<body><h1>Document {{ref}}</h1>')
    else:
        html = '<h1>Document {{ref}}</h1>' + html
    labels = ['AAA'] if mode in ('direct', 'compiled') else ['AAA', 'BBB']
    if mode == 'direct':
        action = lambda: engine.render_pdf(html.replace('{{ref}}', labels[0]), css)
    elif mode == 'compiled':
        compiled = engine.compile_pdf(html.replace('{{ref}}', labels[0]), css)
        action = compiled.render_pdf
    else:
        compiled = engine.compile_pdf(html, css)
        action = (lambda: compiled.render_pdf_bindings({'ref': labels})) if mode == 'fixed' else (
            lambda: compiled.render_pdf_reflow_bindings({'ref': labels}))
    data = bytes(action())
    assert data == bytes(action()), 'PDF changed on repeat rendering'
    reader = PdfReader(BytesIO(data))
    assert len(reader.pages) == len(labels), 'unexpected page count'
    texts = [' '.join(page.extract_text().split()) for page in reader.pages]
    assert all('Document '+label in text for label, text in zip(labels, texts))
    return data, texts, html.replace('{{ref}}', labels[0])


def pixel_hash(png):
    with Image.open(BytesIO(png)) as image:
        rgb = image.convert('RGB')
        return {'size': list(rgb.size), 'sha256': sha256(rgb.tobytes()).hexdigest()}


def check(out, modes=MODES):
    out.mkdir(parents=True, exist_ok=True)
    report = {'schema': 'fullbleed.hidden_text.v1', 'ok': False,
              'versions': {name: metadata.version(name) for name in ('fullbleed', 'pypdf', 'pypdfium2', 'pillow')},
              'scope': __doc__, 'cases': [], 'failures': []}
    for case in cases():
        for mode in modes:
            name = case['name']+'-'+mode
            folder = out/name
            folder.mkdir(exist_ok=True)
            css = BASE+case['css']
            (folder/'input.css').write_text(css, encoding='utf-8')
            row = {'name': name, 'kind': case['kind'], 'mode': mode, 'variants': {}}
            try:
                engine = fullbleed.PdfEngine()
                for variant, html in [('source', case['html']), ('control', case['control'])]:
                    (folder/(variant+'.html')).write_text(html, encoding='utf-8')
                    data, text, concrete = render(engine, html, css, mode)
                    path = folder/(variant+'.pdf')
                    path.write_bytes(data)
                    values = {'pdf_sha256': sha256(data).hexdigest(), 'text': text, 'pdfium': []}
                    with pdfium.PdfDocument(data) as document:
                        for index, page in enumerate(document):
                            bitmap = page.render(scale=1)
                            image = bitmap.to_pil().convert('RGB')
                            values['pdfium'].append({'size': list(image.size), 'sha256': sha256(image.tobytes()).hexdigest()})
                            image.save(folder/f'{variant}-pdfium-{index+1}.png')
                            image.close(); bitmap.close(); page.close()
                    if mode == 'direct':
                        previews = [bytes(png) for png in engine.render_image_pages(concrete, css, 96)]
                        values['native'] = [pixel_hash(png) for png in previews]
                        for index, png in enumerate(previews):
                            (folder/f'{variant}-native-{index+1}.png').write_bytes(png)
                    row['variants'][variant] = values
                row['ok'] = row['variants']['source'] == row['variants']['control']
            except Exception as error:
                row.update(ok=False, error=str(error))
            if not row['ok']:
                report['failures'].append(name)
            report['cases'].append(row)
    report['ok'] = not report['failures']
    (out/'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--direct-only', action='store_true', help='Run only direct rendering for a baseline diagnosis')
    args = parser.parse_args()
    result = check(args.out, ('direct',) if args.direct_only else MODES)
    print(json.dumps({'ok': result['ok'], 'cases': len(result['cases']), 'failures': result['failures']}))
    raise SystemExit(0 if result['ok'] else 1)
