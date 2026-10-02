"""Exercise real finalized-PDF previews through the installed native extension."""
import importlib.util
from pathlib import Path

import pytest
import fullbleed


def test_standard_font_previews_match_explicit_reference_widths(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    path = Path(__file__).resolve().parents[1] / "tools/smoke_base14_preview.py"
    spec = importlib.util.spec_from_file_location("base14_smoke", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    result = module.check(tmp_path)
    assert result["ok"] and len(result["cases"]) == 30
