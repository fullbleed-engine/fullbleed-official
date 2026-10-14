"""Installed-native regression for font-based decoration thickness and replay."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_font_decoration_thickness_in_pdf_compiled_reflow_and_previews(tmp_path):
    if not hasattr(fullbleed, 'PdfEngine'):
        pytest.skip('installed native engine required')
    for dependency in ['fontTools', 'pypdf', 'pypdfium2', 'PIL']:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / 'tools/smoke_text_decoration_thickness.py'
    report = runpy.run_path(str(script))['check'](tmp_path)
    assert report['ok'], report['failures']
