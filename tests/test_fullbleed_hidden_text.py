"""Verify default hidden text and explicit source display in installed output."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_hidden_text_and_author_overrides_in_pdf_previews_and_replay(tmp_path):
    if not hasattr(fullbleed, 'PdfEngine'):
        pytest.skip('installed native engine required')
    for dependency in ['pypdf', 'pypdfium2', 'PIL']:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1]/'tools/smoke_hidden_text.py'
    result = runpy.run_path(str(script))['check'](tmp_path)
    assert result['ok'], result['failures']
