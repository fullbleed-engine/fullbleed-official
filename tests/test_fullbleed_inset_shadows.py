from pathlib import Path
import runpy

import fullbleed
import pytest


def test_inset_shadow_spread_offsets_corners_and_blur(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    script = Path(__file__).resolve().parents[1] / "tools/smoke_inset_shadows.py"
    report = runpy.run_path(str(script))["check"](tmp_path)
    assert report["ok"], [case["name"] for case in report["cases"] if not case["ok"]]
