from __future__ import annotations

import json
from types import SimpleNamespace

from fullbleed_cli import scaffold


def test_invoice_template_tree_contains_report_with_css_metadata_defaults() -> None:
    files = scaffold._load_template_tree("new/invoice")
    assert "report.py" in files
    assert "templates/invoice.html" in files
    assert "templates/invoice.css" in files
    assert "output/.gitkeep" in files
    assert "output/.gitignore" in files
    assert "document_css_href" in files["report.py"]
    assert "document_css_source_path" in files["report.py"]
    assert "document_css_required" in files["report.py"]
    assert "emit_artifacts(" in files["report.py"]


def test_statement_template_tree_contains_report_with_css_metadata_defaults() -> None:
    files = scaffold._load_template_tree("new/statement")
    assert "report.py" in files
    assert "templates/statement.html" in files
    assert "templates/statement.css" in files
    assert "output/.gitkeep" in files
    assert "output/.gitignore" in files
    assert "document_css_href" in files["report.py"]
    assert "document_css_source_path" in files["report.py"]
    assert "document_css_required" in files["report.py"]
    assert "emit_artifacts(" in files["report.py"]


def test_cmd_new_template_invoice_writes_report_scaffold(tmp_path, capsys) -> None:
    args = SimpleNamespace(
        template="invoice",
        path=str(tmp_path),
        force=False,
        json=True,
    )
    scaffold.cmd_new_template(args)
    payload = json.loads(capsys.readouterr().out)
    assert payload["ok"] is True
    assert payload["template"] == "invoice"
    assert (tmp_path / "report.py").exists()
    assert (tmp_path / "templates" / "invoice.html").exists()
    assert (tmp_path / "templates" / "invoice.css").exists()
    assert (tmp_path / "output" / ".gitkeep").exists()
    assert (tmp_path / "output" / ".gitignore").exists()


def test_cmd_new_template_statement_writes_report_scaffold(tmp_path, capsys) -> None:
    args = SimpleNamespace(
        template="statement",
        path=str(tmp_path),
        force=False,
        json=True,
    )
    scaffold.cmd_new_template(args)
    payload = json.loads(capsys.readouterr().out)
    assert payload["ok"] is True
    assert payload["template"] == "statement"
    assert (tmp_path / "report.py").exists()
    assert (tmp_path / "templates" / "statement.html").exists()
    assert (tmp_path / "templates" / "statement.css").exists()
    assert (tmp_path / "output" / ".gitkeep").exists()
    assert (tmp_path / "output" / ".gitignore").exists()
