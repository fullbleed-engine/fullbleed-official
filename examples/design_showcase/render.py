#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Render four designed document families with installed Fullbleed and stdlib."""

from __future__ import annotations

import argparse
from decimal import Decimal
import hashlib
from html import escape
from importlib import metadata, resources
import json
from pathlib import Path
import subprocess
import sys

import fullbleed

ROOT = Path(__file__).resolve().parent
DATA = json.loads((ROOT / 'data.json').read_text(encoding='utf-8'))
FONTS = [Path(str(resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf'))),
         ROOT / 'fonts/BebasNeue-Regular.ttf', ROOT / 'fonts/DMSerifDisplay-Regular.ttf',
         ROOT / 'fonts/DMSerifDisplay-Italic.ttf']


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def money(value):
    return f'${Decimal(value):,.2f}'


def substitute(template, values):
    for key, value in values.items():
        template = template.replace('{{' + key + '}}', str(value))
    return template


def sources(name):
    html = (ROOT / f'{name}.html').read_text(encoding='utf-8')
    css = (ROOT / 'common.css').read_text(encoding='utf-8') + '\n' + (ROOT / f'{name}.css').read_text(encoding='utf-8')
    if name == 'invoice':
        total = Decimal('0')
        rows = []
        for item in DATA['invoice']:
            amount = Decimal(item['rate']) * item['hours']
            total += amount
            rows.append(f"<tr><td><strong>{escape(item['description'])}</strong>"
                        f"<p class='item-note'>{escape(item['detail'])}</p></td>"
                        f"<td class='number'>{item['hours']}</td><td class='number'>{money(item['rate'])}</td>"
                        f"<td class='number'>{money(amount)}</td></tr>")
        html = substitute(html, dict(invoice_rows=''.join(rows), subtotal=money(total), total=money(total)))
    elif name == 'report':
        report = DATA['report']
        allocations = report['allocations']
        total = sum(item['amount'] for item in allocations)
        funding = []
        for item in allocations:
            share = Decimal(item['amount']) / total * 100
            funding.append(f"<tr><td><strong>{escape(item['name'])}</strong></td>"
                           f"<td class='number'>{money(item['amount'])}</td><td class='number share'>{share:.0f}%"
                           f"<div class='share-track' style='margin-left:auto'><div class='share-fill' style='width:{share}%'></div></div></td></tr>")
        bars = ''.join(f"<div class='chart-col'><div class='chart-value'>{value:,}</div>"
                       f"<div class='bar' style='height:{value / 980 * 105:.2f}pt'></div></div>"
                       for value in report['quarterly_households'])
        landscape = (ROOT / 'landscape.svg').read_text(encoding='utf-8').replace('<svg ', '<svg class="landscape" ', 1)
        html = substitute(html, dict(landscape=landscape, households=f"{report['households']:,}",
                          hours=f"{report['volunteer_hours']:,}", spaces=report['spaces'],
                          report_bars=bars, funding_rows=''.join(funding), funding_total=money(total)))
    elif name == 'statement':
        bars = ''.join(f"<div class='chart-col'><div class='chart-value'>${value}</div>"
                       f"<div class='bar' style='height:{value / 250 * 63:.2f}pt'></div></div>"
                       for value in DATA['statements']['monthly_contributions'])
        html = substitute(html, dict(statement_bars=bars))
    return html, css


def member_values(member):
    opening = Decimal(member['opening_balance'])
    return {**{key: member[key] for key in ['member_name', 'address', 'account_id']},
            'opening_balance': money(opening), 'first_balance': money(opening + 125),
            'second_balance': money(opening + 250), 'closing_balance': money(opening + 200),
            'bal': money(opening + 200)}


def invoke(command, log):
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, encoding='utf-8')
    log.write_text(process.stdout, encoding='utf-8')
    log.with_suffix('.stderr.txt').write_text(process.stderr, encoding='utf-8')
    try:
        result = json.loads(process.stdout)
    except json.JSONDecodeError:
        result = dict(ok=False, error=dict(message=process.stderr or process.stdout))
    result['process_exit_code'] = process.returncode
    return result


def ordinary(out, name, html, css, expected_pages, markers, *, profile='none', dpi=120):
    folder = out / name
    folder.mkdir(parents=True, exist_ok=True)
    html_path, css_path, pdf = folder / f'{name}.html', folder / f'{name}.css', folder / f'{name}.pdf'
    html_path.write_text(html, encoding='utf-8')
    css_path.write_text(css, encoding='utf-8')
    args = ['--html', str(html_path), '--css', str(css_path), '--out', str(pdf),
            '--document-title', {'invoice':'Northstar Studio — Invoice NS-1042',
                                 'report':'Common Ground — Community Review 2026',
                                 'notice':'Riverton Services — Planned Water Maintenance',
                                 'statement-proof':'Hillside Collective — Member Statement'}[name],
            '--document-lang', 'en-US', '--pdf-profile', profile,
            '--fail-on', 'overflow', '--fail-on', 'missing-glyphs', '--fail-on', 'font-subst']
    for font in FONTS:
        args += ['--asset', str(font)]
    base = [sys.executable, '-I', '-m', 'fullbleed', '--json-only']
    rendered = invoke(base + ['render', *args, '--emit-image', str(folder / 'preview'),
                             '--image-dpi', str(dpi), '--emit-glyph-report', str(folder / 'glyphs.json'),
                             '--emit-page-data', str(folder / 'page-data.json'),
                             '--emit-jit', str(folder / 'render.jit')], folder / 'render.json')
    verify_args = ['--emit-pdf' if arg == '--out' else arg for arg in args]
    verified = invoke(base + ['verify', *verify_args], folder / 'verify.json')
    first_hash = rendered.get('outputs', {}).get('sha256')
    verified_hash = verified.get('outputs', {}).get('sha256')
    replay_ok = bool(first_hash and first_hash == verified_hash and first_hash == sha(pdf))
    (folder / 'replay-verification.json').write_text(json.dumps(dict(
        ok=replay_ok, method='separate CLI render and verify processes; identical PDF SHA-256',
        render_sha256=first_hash, verify_sha256=verified_hash), indent=2)+'\n', encoding='utf-8')
    inspection = fullbleed.inspect_pdf(str(pdf)) if pdf.exists() else {}
    (folder / 'inspection.json').write_text(json.dumps(inspection, indent=2)+'\n', encoding='utf-8')
    pages = fullbleed.extract_pdf_page_texts(str(pdf)).get('pages', []) if pdf.exists() else []
    text = '\n'.join(page.get('text', '') for page in pages)
    failures = [f'missing text: {marker}' for marker in markers if marker not in text]
    if '{{' in text or '}}' in text:
        failures.append('unresolved template placeholder')
    if not replay_ok:
        failures.append('render/verify PDF hashes differ')
    if inspection.get('page_count') != expected_pages:
        failures.append(f"expected {expected_pages} pages, got {inspection.get('page_count')}")
    for stage, result in [('render', rendered), ('verify', verified)]:
        if not result.get('ok') or result['process_exit_code']:
            failures.append(f"{stage}: {result.get('error') or result.get('message')}")
    return dict(id=name, ok=not failures, failures=failures, page_count=inspection.get('page_count'),
                pdf=str(pdf), sha256=sha(pdf) if pdf.exists() else None,
                bytes=pdf.stat().st_size if pdf.exists() else 0, profile=inspection.get('profile'),
                preview_paths=rendered.get('outputs', {}).get('image_paths', []),
                fallbacks=rendered.get('outputs', {}).get('fallbacks'),
                reproducibility='pass' if replay_ok else 'fail',
                source_html=str(html_path), source_css=str(css_path))


def statements(out, dpi):
    template, css = sources('statement')
    values = [member_values(member) for member in DATA['statements']['members']]
    proof_html = substitute(template, {key:escape(value) for key,value in values[0].items()})
    proof = ordinary(out, 'statement-proof', proof_html, css, 1, ['Jordan Ellis', '$520.00'], dpi=dpi)
    folder = out / 'statements'
    folder.mkdir(parents=True, exist_ok=True)
    (folder / 'statement-template.html').write_text(template, encoding='utf-8')
    (folder / 'statement.css').write_text(css, encoding='utf-8')
    bindings = {key:[value[key] for value in values] for key in values[0]}
    (folder / 'bindings.json').write_text(json.dumps(bindings, indent=2)+'\n', encoding='utf-8')
    engine = fullbleed.PdfEngine(font_files=[str(font) for font in FONTS],
                                document_title='Hillside Collective — Member Statements', document_lang='en-US')
    compiled = engine.compile_pdf(template, css)
    (folder / 'compiled-stats.json').write_text(json.dumps(compiled.stats(), indent=2)+'\n', encoding='utf-8')
    pdf, replay = folder / 'statements.pdf', folder / 'replay.pdf'
    compiled.render_pdf_reflow_bindings_to_file(bindings, str(pdf), compression='compact')
    compiled.render_pdf_reflow_bindings_to_file(bindings, str(replay), compression='compact')
    inspection = fullbleed.inspect_pdf(str(pdf))
    pages = fullbleed.extract_pdf_page_texts(str(pdf)).get('pages', [])
    failures = list(proof['failures'])
    if len(pages) != len(values):
        failures.append(f'expected {len(values)} record pages, got {len(pages)}')
    for page, value in zip(pages, values):
        for key in ['member_name', 'account_id', 'closing_balance']:
            if value[key] not in page.get('text', ''):
                failures.append(f"record {value['account_id']} missing {key}")
        # Letter spacing may produce a separate extraction chunk per glyph.
        compact_text = ''.join(page.get('text', '').split())
        if compact_text.count(value['closing_balance']) < 3:
            failures.append(f"record {value['account_id']} missing a displayed closing balance")
        if '{{' in page.get('text', '') or '}}' in page.get('text', ''):
            failures.append(f"record {value['account_id']} has an unresolved placeholder")
    if sha(pdf) != sha(replay):
        failures.append('compiled replay hash differs')
    previews = list(engine.render_finalized_pdf_image_pages_to_dir(str(pdf), str(folder / 'preview'), dpi, 'statements'))
    (folder / 'inspection.json').write_text(json.dumps(inspection, indent=2)+'\n', encoding='utf-8')
    return dict(id='statements', ok=not failures, failures=failures, pdf=str(pdf), sha256=sha(pdf),
                bytes=pdf.stat().st_size, page_count=inspection.get('page_count'), record_count=len(values),
                preview_paths=previews, reproducibility='pass' if sha(pdf)==sha(replay) else 'fail',
                compiled_stats=compiled.stats(), proof=proof, source_html=str(folder / 'statement-template.html'),
                source_css=str(folder / 'statement.css'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=ROOT / 'output')
    parser.add_argument('--only', choices=['invoice', 'report', 'notice', 'statements'])
    parser.add_argument('--dpi', type=int, default=120)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    results = []
    cases = {'invoice':(1, ['NS-1042', 'Maple & Finch', '$1,870.00']),
             'report':(3, ['Common Ground', '1,240', '$420,000.00', 'What we count.']),
             'notice':(1, ['WATER SUPPLY', '20 OCT', 'RV-1020'])}
    for name, (pages, markers) in cases.items():
        if args.only and args.only != name:
            continue
        html, css = sources(name)
        result = ordinary(out, name, html, css, pages, markers, profile='pdfua1' if name=='notice' else 'none', dpi=args.dpi)
        results.append(result)
        print(json.dumps({key:result[key] for key in ['id','ok','page_count','failures']}), flush=True)
    if not args.only or args.only == 'statements':
        result = statements(out, args.dpi)
        results.append(result)
        print(json.dumps({key:result[key] for key in ['id','ok','page_count','failures']}), flush=True)
    report = dict(schema='fullbleed.design_showcase.v1', ok=all(case['ok'] for case in results),
                  fullbleed_version=metadata.version('fullbleed'), fictional_data=True,
                  checks=['expected page counts','required text and record order','resolved placeholders',
                          'CLI overflow/glyph/font gates on ordinary cases and first-record statement proof',
                          'deterministic replay','internal PDF inspection'],
                  external_standards_validation=False,
                  fonts=[dict(file=font.name, sha256=sha(font)) for font in FONTS], cases=results)
    (out / 'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
