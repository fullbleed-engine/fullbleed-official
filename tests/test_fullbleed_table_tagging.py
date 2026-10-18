"""Read saved table structures and marked content independently of the engine."""
from pathlib import Path
import runpy

import pytest


def test_rich_table_content_has_nonoverlapping_tags_and_preserved_relationships(tmp_path):
    pytest.importorskip('pypdf')
    pytest.importorskip('pypdfium2')
    script = Path(__file__).resolve().parents[1]/'tools/smoke_table_tagging.py'
    report = runpy.run_path(str(script))['check'](tmp_path/'table-tagging')
    failed = [(c['profile'], c['name'], c['mode'], sorted(set(c['errors'])))
              for c in report['cases'] if not c['ok']]
    assert report['ok'] and len(report['cases']) == 180, failed
