"""Check the actual embedded faces and previews emitted by the installed engine."""
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_registered_family_defaults_match_explicit_faces(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    for dependency in ["pypdf", "pypdfium2", "fontTools", "PIL"]:
        pytest.importorskip(dependency)
    script = Path(__file__).resolve().parents[1] / "tools/smoke_font_families.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "families")
    assert report["ok"] and len(report["cases"]) == 54 and report["pages"] == 84
