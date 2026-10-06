"""Verify mixed inline layout through the installed native renderer."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_mixed_inline_wrapping_with_independent_readers(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ["pypdf", "pypdfium2", "PIL"]:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_inline_wrapping.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "inline-wrapping")
    assert report["ok"] and len(report["cases"]) == 40
