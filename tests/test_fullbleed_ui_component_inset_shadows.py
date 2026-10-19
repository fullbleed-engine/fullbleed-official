"""Exercise card styling and both HTML export modes against real PDF output."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_component_inset_shadows_and_html_exports(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("installed native engine required")
    for dependency in ("pypdf", "pypdfium2", "PIL"):
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_component_inset_shadows.py"
    result = runpy.run_path(str(script))["check"](tmp_path)
    assert result["ok"] and len(result["cases"]) == 8 and result["checks"] == 52
