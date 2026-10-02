from pathlib import Path
import runpy
import pytest
import fullbleed


def test_installed_finalized_gradient_previews(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    script = Path(__file__).resolve().parents[1] / "tools" / "smoke_gradient_preview.py"
    report = runpy.run_path(str(script))["check"](tmp_path)
    assert report["ok"]
