from __future__ import annotations

import json
from types import SimpleNamespace

from fullbleed_cli import scaffold


def test_accessible_template_is_registered() -> None:
    tmpl = scaffold.TEMPLATES.get("accessible")
    assert tmpl is not None
    assert tmpl["source_dir"] == "new/accessible"


def test_accessible_template_tree_contains_expected_files() -> None:
    files = scaffold._load_template_tree("new/accessible")
    assert "README.md" in files
    assert "report.py" in files
    assert "styles/report.css" in files
    assert "output/.gitkeep" in files
    assert "output/.gitignore" in files
    assert "fullbleed.ui.accessibility" in files["report.py"]
    assert "fullbleed.accessibility" in files["report.py"]
    assert "AccessibilityEngine" in files["report.py"]
    assert "render_bundle(" in files["report.py"]
    assert "document_css_href" in files["report.py"]
    assert "document_css_source_path" in files["report.py"]
    assert "document_css_required" in files["report.py"]
    assert "accessibility_scaffold_a11y_verify_engine.json" in files["README.md"]
    assert "accessibility_scaffold_pmr_engine.json" in files["README.md"]


def test_cmd_new_template_accessible_writes_scaffold(tmp_path, capsys) -> None:
    args = SimpleNamespace(
        template="accessible",
        path=str(tmp_path),
        force=False,
        json=True,
    )

    scaffold.cmd_new_template(args)
    payload = json.loads(capsys.readouterr().out)

    assert payload["ok"] is True
    assert payload["template"] == "accessible"
    assert (tmp_path / "README.md").exists()
    assert (tmp_path / "report.py").exists()
    assert (tmp_path / "styles" / "report.css").exists()
    assert (tmp_path / "output" / ".gitkeep").exists()
    assert (tmp_path / "output" / ".gitignore").exists()
