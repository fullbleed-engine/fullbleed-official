#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check finalized page-margin geometry in saved PDFs with independent readers.

The explicit overflow fixture intentionally preserves oversized authored widths.
These checks do not promise reflow or fit diagnostics for captured running forms.
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
from pypdf.generic import ContentStream

inspect = runpy.run_path(str(Path(__file__).with_name('smoke_table_tagging.py')))['inspect']
pixels = runpy.run_path(str(Path(__file__).with_name('smoke_table_captions.py')))['pixels']
PROFILES = ('tagged', 'pdfua1', 'pdfua2')
MODES = ('direct', 'compiled', 'repeat', 'reflow', 'compact')
INK = (217, 237, 227)
BASE = '''@page {size:320pt 240pt;margin:40pt 48pt 40pt 24pt}
html,body {font-family:"Fullbleed Preview Mono";font-size:10pt;line-height:12pt;margin:0}
h1,h2 {font-size:8pt;line-height:10pt;margin:0 0 6pt} p {font-size:6pt;line-height:8pt;margin:0}
section+section {break-before:page}'''


def box(kind, content='"X"', extra=''):
    return f'@{kind} {{content:{content};background:#d9ede3;{extra}}}'


def cases():
    corners = ('top-left-corner', 'top-right-corner', 'bottom-left-corner', 'bottom-right-corner')
    top = ('top-left', 'top-center', 'top-right')
    edges = (*top, 'bottom-left', 'bottom-center', 'bottom-right',
             'left-top', 'left-middle', 'left-bottom', 'right-top', 'right-middle', 'right-bottom')
    yield 'corners-control', '', '@page {' + ''.join(box(k) for k in corners) + '}'
    yield 'explicit-side-widths', '', '@page {' + ''.join(
        box(k, extra=f'width:{60 if "left" in k else 12}pt')
        for k in (*edges[6:], *corners)) + '}'
    yield 'explicit-asymmetric', '', '@page {' + ''.join(box(k, extra='width:60pt') for k in edges[:6]) + '}'
    yield 'auto-three', '', '@page {' + ''.join(box(k) for k in top) + '}'
    yield 'auto-two-proportional', '', '@page {' + box('top-left') + box('top-right', '"XXX"') + '}'
    yield 'auto-shrink', '', '@page {size:200pt 240pt;margin-left:80pt;margin-right:80pt;' + box('top-left', '"XX XX"') + box('top-right', '"XXXX XXXX"') + '}'
    yield 'auto-all-sides', '', '@page {' + ''.join(box(k) for k in (*edges, *corners)) + '}'
    yield 'auto-reset', '', '@page {' + box('top-left', extra='width:60pt') + '} @page:first {@top-left {width:auto;width:-20pt}}'
    yield 'running-pair', '<header class="left">Left</header><header class="right">Right</header>', (
        '@page {' + box('top-left', 'element(left)') + box('top-right', 'element(right)') + '}'
        '.left {position:running(left);width:40pt}.right {position:running(right);width:80pt}')
    yield 'named-string', '', ('h1 {string-set:section content()} @page {' +
        box('top-left', 'string(section)') + box('top-right', '"XXX"') + '}')
    yield 'counter-growth', '', ('@page:first {counter-increment:page 9} @page {' +
        box('top-left', '"PAGE" counter(page)') + box('top-right', '"XXXXX"') + '}')
    yield 'named-page', '', ('section.wide {page:wide} @page wide {size:360pt 240pt;margin-left:30pt;margin-right:70pt}' +
        '@page {' + ''.join(box(k, extra='width:60pt') for k in top) + '}')
    yield 'fixed-overflow', '', '@page {' + box('top-left', extra='width:300pt') + box('top-right', extra='width:300pt') + '}'


def expected(name, local, person):
    corners = [[0, 0, 24, 40], [272, 0, 48, 40], [0, 200, 24, 40], [272, 200, 48, 40]]
    top = [[24, 0, 248/3, 40], [24+248/3, 0, 248/3, 40], [24+496/3, 0, 248/3, 40]]
    if name == 'corners-control':
        return 320, corners
    if name == 'explicit-asymmetric':
        return 320, [[x, y, 60, 40] for y in (0, 200) for x in (24, 118, 212)]
    if name == 'explicit-side-widths':
        return 320, [
            [x, 40+i*160/3, w, 160/3] for x,w in ((-36,60), (272,12)) for i in range(3)
        ] + [[x,y,w,40] for x,w in ((-36,60), (272,12)) for y in (0,200)]
    if name == 'auto-three':
        return 320, top
    if name == 'auto-shrink':
        return 200, [[80, 0, 13.5, 40], [93.5, 0, 26.5, 40]]
    if name == 'auto-all-sides':
        return 320, top + [[x, 200, w, h] for x, _, w, h in top] + [
            [x, 40+i*160/3, w, 160/3] for x,w in ((0,24), (272,48)) for i in range(3)] + corners
    if name == 'auto-reset':
        return 320, [[24, 0, 248 if local == 0 else 60, 40]]
    if name == 'named-page':
        return (360, [[x,0,60,40] for x in (30,130,230)]) if local else (320, [[x,0,60,40] for x in (24,118,212)])
    if name == 'fixed-overflow':
        return 320, [[24,0,300,40], [-28,0,300,40]]
    first_width = {'auto-two-proportional': 62, 'running-pair': 248/3,
        'named-string': 124 if person == 'Ada' else 248*8/11,
        'counter-growth': 124 if local == 0 else 248*6/11}[name]
    return 320, [[24,0,first_width,40], [24+first_width,0,248-first_width,40]]


def matrix_product(parent, child):
    a,b,c,d,e,f = parent
    g,h,i,j,k,l = child
    return a*g+c*h, b*g+d*h, a*i+c*j, b*i+d*j, a*k+c*l+e, b*k+d*l+f


def rectangles(page, reader):
    found = []
    identity = (1,0,0,1,0,0)
    def walk(stream, resources, matrix=identity, fill=None, depth=0):
        assert depth < 24, 'Unexpected recursive form'
        states, paths = [], []
        for args, op in ContentStream(stream, reader).operations:
            if op == b'q':
                states.append((matrix, fill))
            elif op == b'Q':
                matrix, fill = states.pop()
            elif op == b'cm':
                matrix = matrix_product(matrix, tuple(float(v) for v in args))
            elif op == b'rg':
                fill = tuple(round(float(v)*255) for v in args)
            elif op in {b'g', b'k'}:
                fill = None
            elif op == b're':
                x,y,w,h = map(float, args)
                a,b,c,d,e,f = matrix
                points = [(a*px+c*py+e, b*px+d*py+f) for px,py in ((x,y),(x+w,y),(x,y+h),(x+w,y+h))]
                left, right = min(p[0] for p in points), max(p[0] for p in points)
                bottom, top = min(p[1] for p in points), max(p[1] for p in points)
                paths.append([left, float(page.mediabox.height)-top, right-left, top-bottom])
            elif op in {b'f', b'F', b'f*', b'B', b'B*', b'b', b'b*', b'S', b's', b'n'}:
                if fill == INK and op not in {b'S', b's', b'n'}:
                    found.extend(paths)
                paths = []
            elif op == b'Do':
                child = resources['/XObject'][args[0]].get_object()
                if child.get('/Subtype') == '/Form':
                    walk(child, child.get('/Resources', resources), matrix_product(matrix, tuple(float(v) for v in child.get('/Matrix', identity))), fill, depth+1)
        assert not states, 'Unbalanced graphics state'
    walk(page.get_contents(), page['/Resources'])
    return sorted(found)


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    font = Path(__file__).resolve().parents[1] / 'src/preview_fonts/FullbleedPreviewMono-Regular.ttf'
    report = dict(schema='fullbleed.page_margin_layout_smoke.v1', ok=False, cases=[],
        versions={n: metadata.version(n) for n in ('fullbleed','pypdf','pypdfium2')}, font_sha256=sha256(font.read_bytes()).hexdigest())
    body = ''.join(f'<section class="{"wide" if i>1 else ""}"><h{1 if i==1 else 2}>{"" if i==1 else f"S{i} "}{{{{person}}}}</h{1 if i==1 else 2}><p>BODY{i} {{{{person}}}}</p></section>' for i in range(1,4))
    options = dict(font_files=[str(font)], document_lang='en-US', document_title='Margin layout', document_timestamp='2026-10-19T00:00:00Z')
    for profile in PROFILES:
        engine = fullbleed.PdfEngine(pdf_profile=profile, **options)
        plain_engine = fullbleed.PdfEngine(**options)
        for name, furniture, extra in cases():
            html = '<!doctype html><html lang="en-US"><head><title>Margin layout</title></head><body><main>' + furniture + body + '</main></body></html>'
            css = BASE + extra
            direct = {person: bytes(engine.render_pdf(html.replace('{{person}}', person), css)) for person in ('Ada','Beatrice')}
            direct_pixels = {person: pixels(data) for person,data in direct.items()}
            plain = bytes(plain_engine.render_pdf(html.replace('{{person}}', 'Ada'), css))
            for mode in MODES:
                folder = out / profile / name / mode
                folder.mkdir(parents=True)
                (folder / 'source.html').write_text(html, encoding='utf-8', newline='\n')
                (folder / 'source.css').write_text(css, encoding='utf-8', newline='\n')
                compiled = None if mode == 'direct' else engine.compile_pdf(html if mode in {'reflow','compact'} else html.replace('{{person}}','Ada'), css)
                def render():
                    if mode == 'direct': return bytes(engine.render_pdf(html.replace('{{person}}','Ada'), css))
                    if mode == 'compiled': return bytes(compiled.render_pdf())
                    if mode == 'repeat': return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({'person':['Ada','Beatrice']}, compression='compact' if mode=='compact' else 'throughput'))
                data = render()
                (folder / 'document.pdf').write_bytes(data)
                result = inspect(data)
                errors = result['errors']
                people = ['Ada','Beatrice'] if mode in {'reflow','compact'} else ['Ada']*(2 if mode=='repeat' else 1)
                reader = PdfReader(io.BytesIO(data))
                if len(reader.pages) != 3*len(people): errors.append('Each record must retain three pages')
                geometry = []
                for index,page in enumerate(reader.pages):
                    record,local = divmod(index,3)
                    if record >= len(people): continue
                    width,wanted = expected(name,local,people[record])
                    actual = rectangles(page,reader)
                    wanted = sorted(wanted)
                    if abs(float(page.mediabox.width)-width)>0.025 or abs(float(page.mediabox.height)-240)>0.025:
                        errors.append(f'Incorrect page geometry on page {index+1}')
                    if len(actual)!=len(wanted) or any(abs(a-b)>0.025 for left,right in zip(actual,wanted) for a,b in zip(left,right)):
                        errors.append(f'Margin rectangles differ from the authored geometry on page {index+1}')
                    text = ' '.join(page.extract_text().split())
                    if f'BODY{local+1} {people[record]}' not in text: errors.append('Body record text was lost')
                    if name=='counter-growth' and f'PAGE{9+local}' not in text: errors.append('Page counter escaped its record')
                    if name=='auto-all-sides' and text.count('X') != 16: errors.append('A margin label was lost')
                    geometry.append(dict(page=index+1, expected=wanted, actual=actual))
                for role,count in (('/H1',1),('/H2',2),('/P',3)):
                    if result['roles'].get(role)!=count*len(people): errors.append('Body structure changed: '+role)
                actual_pixels = pixels(data, folder if profile=='pdfua2' and mode=='direct' else None)
                if actual_pixels != [p for person in people for p in direct_pixels[person]]: errors.append('Batch output differs from individual records')
                if mode=='direct':
                    (folder/'untagged.pdf').write_bytes(plain)
                    if actual_pixels != pixels(plain): errors.append('Tagging changed visible layout')
                if data!=render(): errors.append('Repeated output is not byte deterministic')
                result.update(profile=profile,name=name,mode=mode,ok=not errors,pdf_sha256=sha256(data).hexdigest(),pixels=actual_pixels,geometry=geometry)
                (folder/'inspection.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
                report['cases'].append(result)
    report['ok'] = all(case['ok'] for case in report['cases'])
    (out/'verification.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    args = parser.parse_args()
    result = check(args.out)
    failed = [dict(profile=c['profile'],name=c['name'],mode=c['mode'],errors=sorted(set(c['errors']))) for c in result['cases'] if not c['ok']]
    print(json.dumps(dict(ok=result['ok'],cases=len(result['cases']),failed=failed)))
    raise SystemExit(0 if result['ok'] else 1)
