from __future__ import annotations

import json
from types import SimpleNamespace

from fullbleed_cli import scaffold


def test_reference_template_is_registered() -> None:
    tmpl = scaffold.TEMPLATES.get("reference")
    assert tmpl is not None
    assert tmpl["source_dir"] == "new/reference"


def test_reference_template_tree_contains_canonical_scaffold_files() -> None:
    files = scaffold._load_template_tree("new/reference")
    expected = {
        "README.md",
        "SCAFFOLDING.md",
        "COMPLIANCE.md",
        "report.py",
        "data/reference.json",
        "assets/brand-mark.svg",
        "assets/reference-pattern.svg",
        "components/fb_ui.py",
        "components/primitives.py",
        "components/styles/primitives.css",
        "styles/tokens.css",
        "styles/report.css",
        "output/.gitignore",
    }
    assert expected.issubset(files.keys())
    assert "output/canonical_reference.pdf" not in files
    assert "output/canonical_reference_page1.png" not in files
    assert "paginated_context" in files["report.py"]
    assert "reference.weight" in files["report.py"]
    assert "footer_each" in files["report.py"]
    assert "footer_last" in files["report.py"]
    assert "PaginationPage" in files["report.py"]
    assert "render_pdf_with_page_data" in files["report.py"]
    assert "canonical_reference_page_data.json" in files["report.py"]
    assert "page_plan" in files["data/reference.json"]
    assert "Manual pagination" in files["data/reference.json"]
    assert "page-break-after: always" in files["components/styles/primitives.css"]
    assert "Footer templates rendered by PdfEngine" in files["components/architecture.py"]
    assert "python report.py" in files["README.md"]


def test_cmd_new_template_reference_writes_source_only_scaffold(tmp_path, capsys) -> None:
    args = SimpleNamespace(
        template="reference",
        path=str(tmp_path),
        force=False,
        json=True,
    )

    scaffold.cmd_new_template(args)
    payload = json.loads(capsys.readouterr().out)

    assert payload["ok"] is True
    assert payload["template"] == "reference"
    assert payload["bootstrap_enabled"] is False
    assert (tmp_path / "report.py").exists()
    assert (tmp_path / "data" / "reference.json").exists()
    assert (tmp_path / "components" / "primitives.py").exists()
    assert (tmp_path / "components" / "architecture.py").read_text(encoding="utf-8").find("PaginationPage") >= 0
    assert "footer_each" in (tmp_path / "report.py").read_text(encoding="utf-8")
    assert "page_plan" in (tmp_path / "data" / "reference.json").read_text(encoding="utf-8")
    assert (tmp_path / "components" / "styles" / "visuals.css").exists()
    assert (tmp_path / "assets" / "brand-mark.svg").exists()
    assert (tmp_path / "output" / ".gitignore").exists()
    assert not (tmp_path / "output" / "canonical_reference.pdf").exists()
    assert not (tmp_path / "output" / "canonical_reference_page1.png").exists()
