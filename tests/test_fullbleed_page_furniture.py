"""Page furniture must stay outside the PDF reading order in every emission path."""
from pathlib import Path

import pytest


def test_page_furniture_preserves_body_counters_and_record_isolation(tmp_path, monkeypatch):
    pytest.importorskip('pypdf')
    pytest.importorskip('pypdfium2')
    monkeypatch.syspath_prepend(str(Path(__file__).resolve().parents[1] / 'tools'))
    from smoke_page_furniture import check, documents, MODES, PROFILES
    report = check(tmp_path / 'furniture')
    failed = [(c['profile'], c['name'], c['mode'], sorted(set(c['errors'])))
              for c in report['cases'] if not c['ok']]
    assert len(report['cases']) == len(list(documents())) * len(MODES) * len(PROFILES)
    assert report['ok'], failed
