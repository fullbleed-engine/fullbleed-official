#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify inline intrinsic sizing through installed PDFs and independent readers."""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata, resources
import json
from pathlib import Path
import re

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium


def check(out):
    out.mkdir(parents=True, exist_ok=False)
    font = str(resources.files('fullbleed_assets').joinpath('fonts', 'Inter-Variable.ttf'))
    report = {'ok': False, 'version': metadata.version('fullbleed'), 'checkedAt': datetime.now(timezone.utc).isoformat(),
              'fontSha256': sha256(Path(font).read_bytes()).hexdigest(), 'cases': []}
    try:
        for letter_spacing in [0, .45, 1.5]:
            for display in ['flex', 'inline-block']:
                name = f'{display}-{letter_spacing}'
                folder = out / name
                folder.mkdir()
                if display == 'flex':
                    html = '<div class="row"><span>NORTHSTAR STUDIO / INVOICE NS-1042</span><span>01 / 01</span></div>'
                    expected = 'NORTHSTAR STUDIO / INVOICE NS-1042 01 / 01'
                    rule = '.row {display:flex;justify-content:space-between;gap:12pt}'
                else:
                    html = '<div class="row"><span class="label">TOTAL <em>OPERATING</em> ALLOCATION USD</span> END</div>'
                    expected = 'TOTAL OPERATING ALLOCATION USD END'
                    rule = '.label {display:inline-block} em {font-style:normal;color:#b44123}'
                css = ('@page {size:420pt 160pt;margin:20pt} * {margin:0;padding:0} '
                       'body {font-family:Inter;font-size:8pt;line-height:15pt} '
                       f'.row {{letter-spacing:{letter_spacing}pt}} ' + rule)
                (folder / 'input.html').write_text(html, encoding='utf-8', newline='\n')
                (folder / 'style.css').write_text(css, encoding='utf-8', newline='\n')
                row = {'name': name, 'ok': False}
                report['cases'].append(row)
                try:
                    engine = fullbleed.PdfEngine(font_files=[font])
                    data = bytes(engine.render_pdf(html, css))
                    assert data == bytes(engine.render_pdf(html, css)), 'Repeated PDF bytes differ'
                    pdf = folder / 'document.pdf'
                    pdf.write_bytes(data)
                    row['pdfSha256'] = sha256(data).hexdigest()
                    reader = PdfReader(pdf)
                    assert len(reader.pages) == 1
                    row['pypdfText'] = ' '.join(reader.pages[0].extract_text().split())
                    # Independent readers can insert spaces between widely
                    # tracked letters. Check exact character order here; the
                    # separate wrapping suite checks ordinary word boundaries.
                    assert ''.join(row['pypdfText'].split()) == ''.join(expected.split()), 'pypdf content or order differs'
                    previews = list(engine.render_finalized_pdf_image_pages(str(pdf), 96))
                    assert len(previews) == 1
                    (folder / 'native-1.png').write_bytes(bytes(previews[0]))
                    with pdfium.PdfDocument(data) as document:
                        page = document[0]
                        textpage = page.get_textpage()
                        try:
                            text = ''.join(textpage.get_text_range(i, 1) for i in range(textpage.count_chars()))
                            row['pdfiumText'] = ' '.join(text.split())
                            words = []
                            for match in re.finditer(r'\S+', text):
                                boxes = [textpage.get_charbox(i) for i in range(match.start(), match.end())]
                                words.append({'text': match.group(), 'box': [min(b[0] for b in boxes), min(b[1] for b in boxes),
                                              max(b[2] for b in boxes), max(b[3] for b in boxes)]})
                            row['words'] = words
                            bitmap = page.render(scale=1.5)
                            image = bitmap.to_pil()
                            image.save(folder / 'pdfium-1.png')
                            image.close()
                            bitmap.close()
                        finally:
                            textpage.close()
                            page.close()
                    assert ''.join(row['pdfiumText'].split()) == ''.join(expected.split()), 'PDFium content or order differs'
                    assert ''.join(word['text'] for word in words) == ''.join(expected.split())
                    centers = [(word['box'][1] + word['box'][3]) / 2 for word in words]
                    assert max(centers) - min(centers) < 2, 'A max-content label wrapped despite available room'
                    assert all(19 <= w['box'][0] < w['box'][2] <= 401 for w in words), 'Text leaves the content box'
                    for a, b in zip(words, words[1:]):
                        assert a['box'][2] <= b['box'][0] + .1, 'Adjacent words overlap'
                    row['ok'] = True
                except Exception as error:
                    row['error'] = str(error)
        report['ok'] = all(row['ok'] for row in report['cases'])
        assert report['ok'], '; '.join(f"{r['name']}: {r['error']}" for r in report['cases'] if not r['ok'])
        return report
    finally:
        (out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8', newline='\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    report = check(parser.parse_args().out)
    print(json.dumps({'ok': report['ok'], 'cases': len(report['cases'])}))
