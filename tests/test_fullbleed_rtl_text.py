"""Exercise RTL layout and PDF Unicode maps through the installed engine."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_arabic_text_and_styled_inline_order(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ["pypdf", "pypdfium2", "PIL"]:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_rtl_text.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "rtl")
    assert report["ok"], [check["name"] for check in report["checks"] if not check["passed"]]
