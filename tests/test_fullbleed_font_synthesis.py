"""Check independent CSS synthesis controls in emitted PDFs and previews."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_font_synthesis_controls_in_installed_rendering_paths(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ["pypdf", "pypdfium2", "PIL"]:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_font_synthesis.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "synthesis")
    assert report["ok"] and len(report["cases"]) == 130 and report["pages"] == 221
