"""Read saved PDFs to catch line-height changes caused by inline authoring."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_inline_line_height_and_pagination_with_independent_reader(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ("pypdfium2", "PIL"):
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_inline_line_height.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "inline-line-height")
    assert report["ok"] and len(report["cases"]) == 84
