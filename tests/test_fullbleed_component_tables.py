"""Verify saved component tables through the installed native renderer."""
from pathlib import Path
import runpy

import pytest


def test_component_table_content_survives_pdf_and_static_html_exports(tmp_path):
    pytest.importorskip("pypdf")
    pytest.importorskip("pypdfium2")
    script = Path(__file__).resolve().parents[1] / "tools/smoke_component_tables.py"
    report = runpy.run_path(str(script))["check"](tmp_path / "component-tables")
    failed = [(c["profile"], c["name"], c["mode"], sorted(set(c["errors"])))
              for c in report["cases"] if not c["ok"]]
    assert report["ok"] and len(report["cases"]) == 240, failed
