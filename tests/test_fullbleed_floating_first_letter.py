"""Installed-native regressions for printed drop-cap geometry and replay."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_floating_first_letter_in_direct_compiled_reflow_and_preview(tmp_path):
    if not hasattr(fullbleed, 'PdfEngine'):
        pytest.skip('installed native engine required')
    for dependency in ['pypdfium2', 'PIL']:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / 'tools/smoke_floating_first_letter.py'
    report = runpy.run_path(str(script))['check'](tmp_path)
    assert report['ok'], [case['name'] for case in report['cases'] if not case['ok']]
