from pathlib import Path
import runpy


def test_running_content_matches_separately_rendered_records(tmp_path):
    check = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/smoke_running_reflow.py'))['check']
    result = check(tmp_path / 'running-reflow')
    assert len(result['cases']) == 54
    assert result['ok'], [
        (case['name'], case['profile'], case['mode'], case['errors'])
        for case in result['cases'] if not case['ok']
    ]
