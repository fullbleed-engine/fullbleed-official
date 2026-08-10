from __future__ import annotations

import json
from pathlib import Path

import pytest

from tools import agentdocbench


def test_task_registry_is_neutral_and_covers_document_agent_failures() -> None:
    payload = json.loads(agentdocbench.TASKS_PATH.read_text(encoding="utf-8"))
    tasks = payload["tasks"]
    assert payload["schema"] == "agentdocbench.tasks.v1"
    assert len(tasks) >= 12
    assert len({task["id"] for task in tasks}) == len(tasks)
    prompts = "\n".join(task["prompt"] for task in tasks)
    assert "Fullbleed" not in prompts
    assert {
        "quarterly-report",
        "accessible-document",
        "pdf-template-overlay",
        "personalized-statements-1000",
        "overflow-repair",
        "multilingual-document",
        "deterministic-reproduction",
        "variable-pagination",
    }.issubset({task["id"] for task in tasks})


def test_prepare_isolated_task_and_refuse_overwrite(tmp_path: Path) -> None:
    workspace = tmp_path / "bench"
    result = agentdocbench.prepare(workspace, ["invoice"])
    assert result["schema"] == "agentdocbench.prepare_result.v1"
    assert result["ok"] is True
    task_root = workspace / "invoice"
    assert sorted(path.name for path in task_root.iterdir()) == [
        "TASK.json",
        "inputs",
        "output",
    ]
    task = json.loads((task_root / "TASK.json").read_text(encoding="utf-8"))
    assert task["schema"] == "agentdocbench.task.v1"
    assert (task_root / "inputs" / "invoice.json").is_file()
    with pytest.raises(ValueError, match="absent or empty"):
        agentdocbench.prepare(workspace, ["invoice"])


def test_score_uses_engine_neutral_artifact_checks(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "bench"
    agentdocbench.prepare(workspace, ["invoice"])
    output = workspace / "invoice" / "output" / "document.pdf"
    output.write_bytes(b"%PDF-1.7\nagentdocbench fixture")

    import fullbleed

    monkeypatch.setattr(
        fullbleed,
        "inspect_pdf",
        lambda _path: {"page_count": 1, "profile": {"claims": []}},
        raising=False,
    )
    monkeypatch.setattr(
        fullbleed,
        "extract_pdf_page_texts",
        lambda _path: {
            "pages": [
                {
                    "text": (
                        "Invoice ADB-INV-1042 for Jordan Lee. "
                        "Amount due 1,284.50."
                    )
                }
            ]
        },
        raising=False,
    )
    result = agentdocbench.score(workspace, ["invoice"])
    assert result["schema"] == "agentdocbench.result.v1"
    assert result["ok"] is True
    assert result["metrics"] == {
        "total": 1,
        "passed": 1,
        "failed": 0,
        "reported_submissions": 0,
        "reported_totals": {},
        "first_pass_reported": 0,
        "first_pass_successes": 0,
    }
    assert result["tasks"][0]["visual_review"]["status"] == "pending"
