"""Keep labels separated in the installed component renderer and static export."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_component_flex_spacing_and_static_exports(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("installed native engine required")
    for dependency in ("pypdf", "pypdfium2", "PIL"):
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_component_flex.py"
    result = runpy.run_path(str(script))["check"](tmp_path)
    assert result["ok"], result["cases"]
    assert len(result["cases"]) == 10
