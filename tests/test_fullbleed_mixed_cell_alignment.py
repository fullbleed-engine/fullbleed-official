"""Mixed inline/block content must retain inherited table-cell text alignment."""
import ctypes
from importlib import resources
from pathlib import Path
import runpy

import fullbleed
import pypdfium2 as pdfium
import pypdfium2.raw as raw
import pytest


@pytest.mark.parametrize('align', ['left', 'center', 'right'])
@pytest.mark.parametrize('direction', ['ltr', 'rtl'])
@pytest.mark.parametrize('wrapper', ['{}', '<span>{}</span>', '<a href="https://example.invalid/share">{}</a>'])
@pytest.mark.parametrize('mode', ['direct', 'compiled', 'compact'])
def test_mixed_cell_alignment_matches_explicit_block(tmp_path, align, direction, wrapper, mode):
    font = resources.files('fullbleed_assets').joinpath('fonts/Inter-Variable.ttf')
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    css = (
        '@page{size:220pt 180pt;margin:20pt}body{margin:0;font-family:Inter;font-size:12pt;line-height:18pt}'
        'table{width:180pt;border-collapse:collapse;table-layout:fixed}'
        f'td{{padding:0;height:54pt;text-align:{align};direction:{direction}}}'
        '.track{width:76pt;height:5pt;background:#ccc;margin-top:7pt;margin-left:auto}'
        'a{color:inherit;text-decoration:none}'
    )

    def render(name, content):
        html = '<table><tr><td>' + content + '<div class="track"></div></td></tr></table>'
        if mode == 'direct':
            data = engine.render_pdf(html.replace('{{share}}', '40%'), css)
        elif mode == 'compiled':
            data = engine.compile_pdf(html.replace('{{share}}', '40%'), css).render_pdf()
        else:
            data = engine.compile_pdf(html, css).render_pdf_reflow_bindings({'share':['40%', '40%']}, compression='compact')
        (tmp_path / (name + '.pdf')).write_bytes(bytes(data))
        viewed = pdfium.PdfDocument(bytes(data))
        pages = []
        try:
            assert len(viewed) == (2 if mode == 'compact' else 1)
            for page in viewed:
                text = page.get_textpage()
                try:
                    values = []
                    for index in range(text.count_chars()):
                        char = text.get_text_range(index, 1)
                        if not char or char.isspace():
                            continue
                        x, y = ctypes.c_double(), ctypes.c_double()
                        assert raw.FPDFText_GetCharOrigin(text, index, ctypes.byref(x), ctypes.byref(y))
                        values.append((char, x.value))
                    pages.append(values)
                finally:
                    text.close(); page.close()
        finally:
            viewed.close()
        return pages

    actual = render('inline-before-block', wrapper.format('{{share}}'))
    expected = render('explicit-block', '<div>{{share}}</div>')
    for current, reference in zip(actual, expected):
        start = min(x for _, x in reference)
        if align == 'right':
            assert start > 150, 'The block reference must actually occupy the cell right edge'
        elif align == 'center':
            assert 80 < start < 120, 'The block reference must actually occupy the cell center'
        else:
            assert start == pytest.approx(20, abs=0.001)
        assert [c for c, _ in current] == [c for c, _ in reference]
        for (char, actual_x), (_, expected_x) in zip(current, reference):
            assert actual_x == pytest.approx(expected_x, abs=0.001), f'{char}: inline content lost {align} alignment'


@pytest.mark.parametrize('authoring', ['html', 'components'])
def test_common_ground_percentages_remain_under_share_heading(tmp_path, monkeypatch, authoring):
    showcase = Path(__file__).resolve().parents[1] / 'examples/design_showcase'
    original = runpy.run_path(str(showcase / 'render.py'))
    if authoring == 'html':
        html, css = original['sources']('report')
    else:
        monkeypatch.syspath_prepend(str(showcase))
        component = runpy.run_path(str(showcase / 'report_components.py'))
        document = component['common_ground_report'](original['DATA']['report'])
        source = document.compile(target='pdf')
        html, css = source.html, source.css
    engine = fullbleed.PdfEngine(font_files=[str(p) for p in original['FONTS']])
    data = bytes(engine.render_pdf(html, css))
    (tmp_path / 'common-ground.pdf').write_bytes(data)
    viewed = pdfium.PdfDocument(data)
    try:
        assert len(viewed) == 3
        page = viewed[2]
        text = page.get_textpage()
        try:
            def bounds(label):
                search = text.search(label, match_case=True)
                try:
                    found = search.get_next()
                    assert found is not None, label
                    assert search.get_next() is None, f'Expected unique label {label}'
                finally:
                    search.close()
                start, count = found
                boxes = [text.get_charbox(i) for i in range(start, start + count)]
                return min(b[0] for b in boxes), max(b[2] for b in boxes)

            heading_left, heading_right = bounds('SHARE')
            for share in ('40%', '30%', '20%', '10%'):
                left, right = bounds(share)
                assert left >= heading_left - 2, f'{share} escaped the share column'
                assert right == pytest.approx(heading_right, abs=2), f'{share} lost right alignment'
        finally:
            text.close(); page.close()
    finally:
        viewed.close()
