from pathlib import Path
import runpy


def test_page_margin_geometry_survives_saved_pdf_emission(tmp_path):
    check = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/smoke_page_margin_layout.py'))['check']
    result = check(tmp_path / 'margin-layout')
    assert len(result['cases']) == 195
    assert result['ok'], [
        (c['profile'], c['name'], c['mode'], c['errors']) for c in result['cases'] if not c['ok']
    ]
