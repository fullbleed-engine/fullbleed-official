from pathlib import Path
import runpy

import fullbleed
import pytest


def test_filtered_descendants_preserve_ancestor_clips_and_stacking(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    script = Path(__file__).resolve().parents[1] / "tools/smoke_filtered_overflow.py"
    report = runpy.run_path(str(script))["check"](tmp_path)
    assert report["ok"], [case["name"] for case in report["cases"] if not case["ok"]]
