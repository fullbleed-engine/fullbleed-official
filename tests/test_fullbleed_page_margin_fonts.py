"""Check supplied page-font faces in actual installed PDFs and both previews."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_page_margin_aliases_match_explicit_registered_faces(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ["pypdf", "pypdfium2", "PIL"]:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_page_margin_fonts.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "page-margin-fonts")
    assert report["ok"] and report["pairs"] == 32 and report["pages"] == 126
