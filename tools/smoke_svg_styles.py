#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Retain bounded inline SVG cascade checks against independently specified colors.

Run with a built and installed wheel. Optional test dependencies are pypdf,
pypdfium2 and Pillow. PDFs are sampled through independent PDFium; native
previews, repeated output, direct/compiled/fixed/reflow and form/raster paths are
also checked. This is not SVG or browser conformance certification.
"""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from PIL import Image
import pypdfium2 as pdfium
from pypdf import PdfReader

BASE = '@page {size:300pt 200pt;margin:0} html,body {margin:0;padding:0} svg {display:block} p {margin:0}'
RED, BLUE, BLACK = '#d00000', '#0060c0', '#000000'
def svg(content='', attributes='', rect_attributes=''):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" {attributes}>'
            + content + f'<rect id="target" class="mark" width="100" height="100" {rect_attributes}/></svg>')


def original_cases():
    rule = f'.mark {{fill:{RED}}}'
    def group(content, attributes=''):
        return f'<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" {attributes}>'+content+'</svg>'
    rect = '<rect width="100" height="100" fill="#d00000"/>'
    return [
        ('explicit-presentation', svg(rect_attributes=f'fill="{RED}"'), '', RED),
        ('document-class', svg(), rule, RED),
        ('document-ancestor', '<main class="brand">'+svg()+'</main>', f'.brand .mark {{fill:{RED}}}', RED),
        ('document-child', svg(), f'svg > rect {{fill:{RED}}}', RED),
        ('document-attribute', svg(), f'rect[id="target"] {{fill:{RED}}}', RED),
        ('document-position', svg(), f'rect:first-of-type {{fill:{RED}}}', RED),
        ('document-inherited-paint', '<main class="brand">'+svg()+'</main>', f'.brand {{fill:{RED}}}', RED),
        ('document-presentation-priority', svg(rect_attributes=f'fill="{BLUE}"'), rule, RED),
        ('document-inline-priority', svg(rect_attributes=f'style="fill:{BLUE}"'), rule, BLUE),
        ('document-important-priority', svg(rect_attributes=f'style="fill:{BLUE}"'), rule.replace('}', '!important}'), RED),
        ('inline-important-priority', svg(rect_attributes=f'style="fill:{BLUE} !important"'), rule.replace('}', '!important}'), BLUE),
        ('svg-local-style', svg('<style>'+rule+'</style>'), '', RED),
        ('svg-screen-style', svg('<style media="screen">'+rule+'</style>'), '', BLACK),
        ('svg-plain-text-style', svg('<style type="text/plain">'+rule+'</style>'), '', BLACK),
        ('svg-print-style', svg('<style media="print">'+rule+'</style>'), '', RED),
        ('svg-nested-screen-media', svg('<style>@media screen {'+rule+'}</style>'), '', BLACK),
        ('document-source-order', svg('<style>.mark {fill:'+BLUE+'}</style>'), rule, RED),
        ('document-preferred-set', '<style title="preferred">.outside {color:blue}</style>'+svg('<style title="alternate">'+rule+'</style>'), '', BLACK),
        ('svg-currentcolor', svg(rect_attributes='fill="currentColor"'), f'svg {{color:{RED}}}', RED),
        ('svg-opacity-replacement', svg(rect_attributes=f'fill="{RED}" fill-opacity="0.25" style="fill-opacity:1"'), '', RED),
        ('svg-important-opacity', svg('<style>.mark {fill-opacity:1 !important}</style>', rect_attributes=f'fill="{RED}" style="fill-opacity:0.25"'), '', RED),
        ('svg-cdata', svg('<style><![CDATA['+rule+']]></style>'), '', RED),
        ('group-fill-opacity-replacement', group('<g fill-opacity="0.25">'+rect.replace('/>', ' fill-opacity="1"/>')+'</g>'), '', RED),
        ('group-opacity-overlap', group('<g opacity="0.5">'+rect+rect+'</g>'), '', '#e78080'),
        ('nested-group-opacity', group('<g opacity="0.5"><g opacity="0.5">'+rect+'</g></g>'), '', '#f3c0c0'),
        ('svg-root-opacity-once', group(rect, 'style="opacity:0.5"'), '', '#e78080'),
        ('svg-local-currentcolor', svg(rect_attributes='fill="currentColor"', attributes='style="color:#d00000"'), '', RED),
        ('svg-paint-alpha-replacement', svg(rect_attributes='fill="rgba(0,96,192,0.25)" style="fill:#d00000"'), '', RED),
        ('svg-invalid-opacity-fallback', svg(rect_attributes='fill="#d00000" style="fill-opacity:0.5;fill-opacity:invalid"'), '', '#e78080'),
        ('svg-stop-important-priority', group('<defs><style>stop {stop-color:#d00000 !important}</style><linearGradient id="paint"><stop offset="0" style="stop-color:#0060c0"/><stop offset="1" style="stop-color:#0060c0"/></linearGradient></defs><rect width="100" height="100" fill="url(#paint)"/>'), '', RED),
    ]


def cases():
    yield from original_cases()
    yield ('document-custom-property', '<main class="brand">'+svg()+'</main>', '.brand {--ink:#d00000} .mark {fill:var(--ink)}', RED)
    yield ('document-invalid-variable', svg(attributes='fill="#d00000"'), 'svg {--bad:invalid} .mark {fill:#0060c0;fill:var(--bad)}', RED)
    yield ('svg-symbol-opacity', '<svg width="100" height="100"><defs><symbol id="shape" viewBox="0 0 100 100" opacity="0.5"><rect width="100" height="100" fill="#d00000"/><rect width="100" height="100" fill="#d00000"/></symbol></defs><use href="#shape" width="100" height="100"/></svg>', '', '#e78080')
    yield ('svg-root-inherit-opacity', '<main style="opacity:0.5">'+svg(attributes='opacity="inherit"', rect_attributes='fill="#d00000"')+'</main>', '', '#f3c0c0')
    yield ('document-layer-normal', svg(), '@layer first,second; @layer first {.mark {fill:#0060c0}} @layer second {.mark {fill:#d00000}}', RED)
    yield ('document-layer-important', svg(), '@layer first,second; @layer first {.mark {fill:#d00000 !important}} @layer second {.mark {fill:#0060c0 !important}}', RED)
    yield ('document-layer-revert', svg(), '@layer first,second; @layer first {.mark {fill:#d00000}} @layer second {.mark {fill:#0060c0;fill:revert-layer}}', RED)
    yield ('svg-currentcolor-child', svg(attributes='color="#d00000" fill="currentColor"', rect_attributes='color="#0060c0"'), '', BLUE)
    yield ('svg-fill-initial', svg(attributes='fill="#d00000"', rect_attributes='style="fill:initial"'), '', BLACK)
    yield ('svg-fill-unset', svg(attributes='fill="#d00000"', rect_attributes='style="fill:unset"'), '', RED)
    yield ('svg-root-opacity-attribute', svg(attributes='opacity="0.5"', rect_attributes='fill="#d00000"'), '', '#e78080')
    yield ('svg-stroke-opacity-replacement', svg(rect_attributes='fill="none" stroke="#d00000" stroke-width="100" stroke-opacity="0.25" style="stroke-opacity:1"'), '', RED)
    yield ('svg-use-inheritance', '<svg width="100" height="100"><defs><rect id="shape" width="100" height="100"/></defs><use href="#shape"/></svg>', 'use {fill:#d00000}', RED)
    yield ('svg-sibling-selector', svg('<g/>'), 'svg > g + rect {fill:#d00000}', RED)
    from base64 import b64encode
    source=svg('<style>.mark {fill:#0060c0}</style>')
    encoded=b64encode(source.encode()).decode()
    yield ('external-svg-isolation', f'<img width="100" height="100" src="data:image/svg+xml;base64,{encoded}">', '.mark {fill:#d00000 !important}', BLUE)


def sample(picture, expected):
    pixel=list(picture.convert('RGB').getpixel((24,24)))
    target=[int(expected[i:i+2],16) for i in (1,3,5)]
    return {'rgb':pixel,'expected':target,'ok':max(abs(a-b) for a,b in zip(pixel,target)) <= 2}


def check(out: Path):
    out.mkdir(parents=True,exist_ok=True)
    records=[]
    failures=[]
    variants=[('vector',False,False),('form',True,False)]
    if fullbleed.build_features().get('svg_raster'):
        variants.append(('raster',True,True))
    for case,source,css,expected in cases():
        for variant,form,raster in variants:
            # An unused filter definition deliberately selects the optional
            # raster path without changing the independently expected paint.
            html=source
            if raster and '<svg ' in html:
                at=html.index('>', html.index('<svg '))+1
                html=html[:at]+'<defs><filter id="force-raster"><feGaussianBlur stdDeviation="0"/></filter></defs>'+html[at:]
            html+=' <p>{{number}}</p>'
            engine=fullbleed.PdfEngine(svg_form_xobjects=form,svg_raster_fallback=raster,document_timestamp='2026-10-16T00:00:00Z')
            css=BASE+css if not css.startswith(BASE) else css
            compiled=engine.compile_pdf(html,css)
            for mode in ('direct','compiled','fixed','reflow'):
                if mode=='direct': action=lambda: engine.render_pdf(html.replace('{{number}}','1042'),css)
                elif mode=='compiled': action=engine.compile_pdf(html.replace('{{number}}','1042'),css).render_pdf
                elif mode=='fixed': action=lambda: compiled.render_pdf_bindings({'number':['1042','1043']})
                else: action=lambda: compiled.render_pdf_reflow_bindings({'number':['1042','1043']})
                pdf=bytes(action()); repeat=bytes(action())
                folder=out/case/variant/mode; folder.mkdir(parents=True,exist_ok=True)
                (folder/'input.html').write_text(html,encoding='utf-8')
                (folder/'input.css').write_text(css,encoding='utf-8')
                (folder/'output.pdf').write_bytes(pdf)
                samples=[]
                with closing(pdfium.PdfDocument(pdf)) as document:
                    for index in range(len(document)):
                        with closing(document[index]) as page, closing(page.render(scale=96/72)) as bitmap:
                            picture=bitmap.to_pil(); picture.save(folder/f'page-{index+1}.png')
                            samples.append(sample(picture,expected))
                pages=1 if mode in ('direct','compiled') else 2
                labels=['1042'] if pages==1 else ['1042','1043']
                text=[' '.join((page.extract_text() or '').split()) for page in PdfReader(BytesIO(pdf)).pages]
                record={'case':case,'variant':variant,'mode':mode,'pages':len(samples),'deterministic':pdf==repeat,
                        'sha256':sha256(pdf).hexdigest(),'samples':samples,'text':text}
                record['ok']=len(samples)==pages and pdf==repeat and text==labels and all(item['ok'] for item in samples)
                records.append(record)
                if not record['ok']: failures.append(record)
            previews=list(engine.render_image_pages(html.replace('{{number}}','1042'),css,96))
            with Image.open(BytesIO(previews[0])) as picture:
                preview=sample(picture,expected)
            repeat_previews=list(engine.render_image_pages(html.replace('{{number}}','1042'),css,96))
            (out/case/variant/'preview.png').write_bytes(previews[0])
            record={'case':case,'variant':variant,'mode':'native-preview','samples':[preview],
                    'deterministic':previews==repeat_previews,'ok':len(previews)==1 and preview['ok'] and previews==repeat_previews}
            records.append(record)
            if not record['ok']: failures.append(record)
    report={'version':version('fullbleed'),'module':fullbleed.__file__,'features':dict(fullbleed.build_features()),
            'cases':len(list(cases())),'checks':len(records),'ok':not failures,'failures':failures,'records':records}
    (out/'verification.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    report=check(args.out)
    print(json.dumps({k:v for k,v in report.items() if k!='records'}))
    raise SystemExit(0 if report['ok'] else 1)
