from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
EXAMPLE_ROOT = REPO_ROOT / "examples" / "agent_workflows"


def _module():
    spec = importlib.util.spec_from_file_location(
        "fullbleed_agent_workflows", EXAMPLE_ROOT / "run_examples.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_agent_examples_are_data_driven_complete_workflows() -> None:
    invoice = json.loads((EXAMPLE_ROOT / "data" / "invoice.json").read_text())
    report = json.loads((EXAMPLE_ROOT / "data" / "report.json").read_text())
    source = (EXAMPLE_ROOT / "run_examples.py").read_text(encoding="utf-8")
    assert invoice["invoice"] == "INV-2026-1042"
    assert report["segment_count"] >= 12
    assert "render_finalized_pdf_image_pages_to_dir" in source
    assert "inspect_pdf" in source
    assert "render_pdf_bindings_to_file" in source
    assert "render_pdf_reflow_bindings_to_file" in source
    assert "pdfua1" in source


def test_agent_example_result_contract_is_stable(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _module()

    def result(name: str) -> dict[str, object]:
        return {"id": name, "ok": True, "page_count": 1}

    for name in (
        "invoice",
        "business_report",
        "accessible",
        "compiled_vdp",
        "compiled_reflow",
    ):
        monkeypatch.setattr(module, name, lambda _out, name=name: result(name))
    monkeypatch.setattr(module.metadata, "version", lambda _name: "9.8.7")
    payload = module.run_all(tmp_path / "out")
    assert payload["schema"] == "fullbleed.agent_examples_result.v1"
    assert payload["fullbleed_version"] == "9.8.7"
    assert payload["metrics"] == {
        "total": 5,
        "passed": 5,
        "failed": 0,
        "pages": 5,
    }
