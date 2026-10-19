from pathlib import Path
import runpy


def test_deferred_running_content_uses_final_slots_and_current_records(tmp_path):
    check = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/smoke_running_layout.py'))['check']
    result = check(tmp_path / 'running-layout')
    assert len(result['cases']) == 120
    assert result['ok'], [
        (c['name'], c.get('profile'), c.get('mode'), c['errors'])
        for c in (*result['cases'], *result['diagnostics']) if not c['ok']
    ]
