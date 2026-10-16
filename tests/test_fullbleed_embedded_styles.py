"""Embedded CSS integration checks need a built and installed native engine."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_embedded_styles_in_installed_pdf_output(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("installed native engine required")
    for dependency in ("pypdf", "pypdfium2", "PIL"):
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_embedded_styles.py"
    result = runpy.run_path(str(script))["check"](tmp_path)
    assert result["ok"], result["failures"]
