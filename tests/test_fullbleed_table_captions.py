"""Caption regression checks use independent saved-PDF readers."""
from pathlib import Path

import pytest


def test_table_captions_keep_ownership_order_text_and_paint_across_emission_modes(tmp_path, monkeypatch):
    pytest.importorskip('pypdf')
    pytest.importorskip('pypdfium2')
    monkeypatch.syspath_prepend(str(Path(__file__).resolve().parents[1] / 'tools'))
    from smoke_table_captions import check, documents, MODES, PROFILES
    report = check(tmp_path/'captions')
    failed = [(c['profile'], c['name'], c['mode'], sorted(set(c['errors'])))
        for c in report['cases'] if not c['ok']]
    assert len(report['cases']) == len(list(documents())) * len(MODES) * len(PROFILES)
    assert report['ok'], failed
