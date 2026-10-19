from pathlib import Path
import runpy


def test_managed_page_margin_paint_checks_saved_pdf_clipping(tmp_path):
    check = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/smoke_page_margin_paint.py'))['check']
    result = check(tmp_path / 'page-margin-paint')
    assert len(result['cases']) == 57
    assert result['ok'], [row for row in result['cases'] if not row['ok']]
