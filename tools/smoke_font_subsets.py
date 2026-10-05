#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check embedded subset outlines, metrics, extraction and pixels with independent readers.

Development-only dependencies: pypdf, pypdfium2, fonttools, pillow. No runtime dependencies
are added to Fullbleed. A baseline records actual output from an earlier installed wheel.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from html import escape
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path
import re
import struct
import sys

import fullbleed
from fontTools.ttLib import TTFont
import pypdfium2 as pdfium
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'benchmarks/pdf_renderers'))
import fixtures
from validate import check_pdf


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def compact(text: str) -> str:
    return re.sub(r'\s+', '', text)


def cases():
    for name in fixtures.NAMES:
        html, contract = fixtures.document(name)
        yield name, fixtures.FONT, html, fixtures.stylesheet(), contract, {}
    fonts = resources.files('fullbleed_assets').joinpath('fonts')
    samples = [
        ('inter-variable', 'Inter-Variable.ttf', 'Inter', 'Invoice INV-2042 Résumé café Ångström Ω Ж 123.45', {}),
        ('noto-variable', 'NotoSans-Regular.ttf', 'Noto Sans', 'Résumé café Ångström Ā Ć Ω Ж 123.45', {}),
        ('math-static', 'NotoSansMath-Regular.ttf', 'Noto Sans Math', 'A 𝒜 ∑ ∫ ≠ → 123.45', {}),
        ('symbols-static', 'NotoSansSymbols-Regular.ttf', 'Noto Sans Symbols', '♀ →', {}),
        ('symbols2-static', 'NotoSansSymbols2-Regular.ttf', 'Noto Sans Symbols2', '☀ ★ ☑ 🂡 ⚀', {}),
        ('inter-unshaped', 'Inter-Variable.ttf', 'Inter', 'Invoice café déjà vu 123.45',
         {'shape_text': False}),
    ]
    for name, filename, family, text, options in samples:
        html = f'<!doctype html><html lang="en"><body><p>{escape(text)}</p></body></html>'
        css = f'@page {{size:A4;margin:36pt}} body {{font-family:"{family}";font-size:18pt}}'
        yield name, Path(str(fonts.joinpath(filename))), html, css, {'text': text}, options


def fonts_in(resources_dict, seen=None):
    seen = set() if seen is None else seen
    resources_dict = resources_dict.get_object()
    fonts = resources_dict.get('/Font', {})
    fonts = fonts.get_object() if hasattr(fonts, 'get_object') else fonts
    for ref in fonts.values():
        font = ref.get_object()
        identity = getattr(ref, 'idnum', id(font))
        if identity in seen:
            continue
        seen.add(identity)
        for descendant in font.get('/DescendantFonts', [font]):
            descendant = descendant.get_object()
            descriptor = descendant.get('/FontDescriptor')
            if descriptor and '/FontFile2' in descriptor.get_object():
                yield font, descendant, descriptor.get_object()['/FontFile2'].get_object()
    xobjects = resources_dict.get('/XObject', {})
    xobjects = xobjects.get_object() if hasattr(xobjects, 'get_object') else xobjects
    for ref in xobjects.values():
        obj = ref.get_object()
        if obj.get('/Subtype') == '/Form' and '/Resources' in obj:
            yield from fonts_in(obj['/Resources'], seen)


def used_glyphs(font, descendant, original):
    used = {0}
    if '/W' in descendant:
        widths = descendant['/W']
        index = 0
        while index < len(widths):
            first = int(widths[index])
            value = widths[index + 1]
            if isinstance(value, list):
                used.update(range(first, first + len(value)))
                index += 2
            else:
                used.update(range(first, int(value) + 1))
                index += 3
    else:
        assert font['/Subtype'] == '/TrueType' and font['/Encoding'] == '/WinAnsiEncoding'
        cmap = original.getBestCmap()
        for code in range(256):
            try:
                character = bytes([code]).decode('cp1252')
            except UnicodeDecodeError:
                continue
            if ord(character) in cmap:
                used.add(original.getGlyphID(cmap[ord(character)]))
    order = original.getGlyphOrder()
    pending = list(used)
    while pending:
        gid = pending.pop()
        glyph = original['glyf'][order[gid]]
        if glyph.isComposite():
            for component in glyph.components:
                child = original.getGlyphID(component.glyphName)
                if child not in used:
                    used.add(child)
                    pending.append(child)
    return used


def check_font(font, descendant, stream, source, legacy):
    data = stream.get_data()
    original = TTFont(source)
    embedded = TTFont(BytesIO(data), checkChecksums=2)
    assert embedded['maxp'].numGlyphs == original['maxp'].numGlyphs
    assert embedded.reader['name'] == original.reader['name'], 'Font notices/names changed'
    padded = data + b'\0' * ((-len(data)) % 4)
    assert sum(struct.unpack(f'>{len(padded) // 4}I', padded)) & 0xffffffff == 0xb1b0afba
    keep = used_glyphs(font, descendant, original)
    before_order, after_order = original.getGlyphOrder(), embedded.getGlyphOrder()
    before_glyf, after_glyf = original.reader['glyf'], embedded.reader['glyf']
    before_loca, after_loca = original['loca'].locations, embedded['loca'].locations
    for gid in keep:
        before = before_glyf[before_loca[gid]:before_loca[gid + 1]]
        after = after_glyf[after_loca[gid]:after_loca[gid + 1]]
        assert before.rstrip(b'\0') == after.rstrip(b'\0'), f'Outline/instructions changed: {gid}'
        assert original['hmtx'][before_order[gid]] == embedded['hmtx'][after_order[gid]], f'Metrics changed: {gid}'
    for old_table, new_table in zip(original['cmap'].tables, embedded['cmap'].tables, strict=True):
        assert (old_table.platformID, old_table.platEncID, old_table.format) == (
            new_table.platformID, new_table.platEncID, new_table.format)
        if not hasattr(old_table, 'cmap'):
            continue
        for point, name in old_table.cmap.items():
            gid = original.getGlyphID(name)
            if gid in keep:
                assert point in new_table.cmap, f'Character mapping lost: {point}'
                assert embedded.getGlyphID(new_table.cmap[point]) == gid
    if not legacy:
        assert embedded['post'].formatType == 3 and len(embedded.reader['post']) == 32
        fields = ['advanceWidthMax', 'minLeftSideBearing', 'minRightSideBearing', 'xMaxExtent']
        original_header = tuple(getattr(embedded['hhea'], name) for name in fields)
        embedded['hhea'].recalc(embedded)
        assert original_header == tuple(getattr(embedded['hhea'], name) for name in fields)
        long_count = embedded['hhea'].numberOfHMetrics
        shared = any(gid >= long_count for gid in keep)
        for gid in range(embedded['maxp'].numGlyphs):
            if gid in keep:
                continue
            advance, bearing = embedded['hmtx'][after_order[gid]]
            assert bearing == 0
            if gid < long_count - 1 or (gid == long_count - 1 and not shared):
                assert advance == 0
    result = {'original_bytes': source.stat().st_size, 'program_bytes': len(data),
              'encoded_bytes': len(stream._data), 'retained_glyphs_and_components': len(keep),
              'tables': {tag: embedded.reader.tables[tag].length for tag in embedded.reader.keys()},
              'original_glyph_ids_outlines_metrics_and_mappings_preserved': True,
              'font_name_and_notice_table_preserved': True}
    original.close()
    embedded.close()
    return result


def run(args):
    args.out.mkdir(parents=True, exist_ok=False)
    baseline = json.loads((args.baseline / 'verification.json').read_text()) if args.baseline else None
    report = {'schema': 'fullbleed.font_subset_smoke.v1', 'ok': False,
              'checked_at': datetime.now(timezone.utc).isoformat(),
              'versions': {name: metadata.version(name) for name in ['fullbleed', 'pypdf', 'pypdfium2', 'fonttools', 'pillow']},
              'baseline': str(args.baseline) if args.baseline else None,
              'legacy_metadata_allowed': args.allow_legacy_metadata, 'cases': [],
              'scope': 'Retained fixtures and independent reader/font checks; not a general rendering or conformance guarantee.'}
    try:
        for name, font_path, html, css, contract, options in cases():
            folder = args.out / name
            folder.mkdir()
            (folder / 'source.html').write_text(html, encoding='utf-8')
            (folder / 'source.css').write_text(css, encoding='utf-8')
            engine = fullbleed.PdfEngine(font_files=[str(font_path)],
                                        document_timestamp='2026-10-05T01:10:00Z', **options)
            pdf = bytes(engine.render_pdf(html, css))
            path = folder / 'output.pdf'
            path.write_bytes(pdf)
            assert pdf == bytes(engine.render_pdf(html, css)), f'{name}: repeat bytes changed'
            reader = PdfReader(path)
            extracted = ''.join(page.extract_text() for page in reader.pages)
            if 'text' in contract:
                assert compact(extracted) == compact(contract['text']), (name, extracted)
            else:
                check = check_pdf(path, contract, fixtures.TOKEN)
                assert check['passed'], check['failures']
            checked_fonts = []
            seen = set()
            for page in reader.pages:
                for font, descendant, stream in fonts_in(page['/Resources'], seen):
                    checked_fonts.append(check_font(font, descendant, stream, font_path, args.allow_legacy_metadata))
            assert checked_fonts, f'{name}: no embedded fonts checked'
            pixels = []
            pdfium_text = []
            with pdfium.PdfDocument(path) as document:
                for index in range(len(document)):
                    page = document[index]
                    text = page.get_textpage()
                    pdfium_text.append(text.get_text_range())
                    text.close()
                    bitmap = page.render(scale=1.25)
                    image = bitmap.to_pil()
                    image.save(folder / f'pdfium-{index + 1}.png')
                    pixels.append(digest(image.tobytes()))
                    bitmap.close()
                    page.close()
            assert compact(''.join(pdfium_text)) == compact(extracted), f'{name}: readers disagree'
            native = [bytes(png) for png in engine.render_finalized_pdf_image_pages(str(path), 96)]
            for index, png in enumerate(native, 1):
                (folder / f'native-{index}.png').write_bytes(png)
            item = {'name': name, 'pdf_bytes': len(pdf), 'pdf_sha256': digest(pdf),
                    'font_source_sha256': digest(font_path.read_bytes()),
                    'html_sha256': digest(html.encode()), 'css_sha256': digest(css.encode()),
                    'pages': len(reader.pages), 'text': extracted, 'pdfium_pixel_sha256': pixels,
                    'native_png_sha256': [digest(png) for png in native], 'fonts': checked_fonts,
                    'repeat_pdf_bytes_identical': True, 'independent_text_readers_agree': True}
            if baseline:
                prior = next(case for case in baseline['cases'] if case['name'] == name)
                for key in ['font_source_sha256', 'html_sha256', 'css_sha256', 'pages', 'text', 'pdfium_pixel_sha256', 'native_png_sha256']:
                    assert item[key] == prior[key], f'{name}: changed {key}'
                assert item['pdf_bytes'] < prior['pdf_bytes'], f'{name}: PDF did not shrink'
                item['baseline_pdf_bytes'] = prior['pdf_bytes']
                item['pdf_reduction_percent'] = round(100 * (1 - len(pdf) / prior['pdf_bytes']), 2)
                item['baseline_text_and_pixels_identical'] = True
            report['cases'].append(item)
            print(json.dumps({'case': name, 'bytes': len(pdf), 'pages': item['pages'], 'fonts_checked': len(checked_fonts)}), flush=True)
        report['ok'] = True
    finally:
        (args.out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--allow-legacy-metadata', action='store_true')
    args = parser.parse_args()
    result = run(args)
    print(json.dumps({'ok': result['ok'], 'cases': len(result['cases'])}))
