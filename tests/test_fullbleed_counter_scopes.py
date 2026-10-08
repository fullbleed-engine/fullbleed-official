"""Counter values must survive actual PDF emission and batch compilation."""
from pathlib import Path
import runpy


def test_counter_scopes_in_installed_pdf_paths(tmp_path):
    tool = Path(__file__).resolve().parents[1] / "tools/smoke_counter_scopes.py"
    result = runpy.run_path(str(tool))["check"](tmp_path / "counters")
    assert result["ok"] and result["cases"] == 12 and len(result["artifacts"]) == 48
