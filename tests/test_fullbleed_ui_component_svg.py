"""Exercise authoring, SVG paint and exported HTML together in real PDFs."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_component_svg_styles_and_html_exports(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("installed native engine required")
    for dependency in ("pypdf", "pypdfium2", "PIL"):
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_component_svg.py"
    result = runpy.run_path(str(script))["check"](tmp_path)
    assert result["ok"] and result["checks"] == 12
