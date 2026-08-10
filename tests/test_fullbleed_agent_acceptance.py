from __future__ import annotations

import json
from pathlib import Path

import pytest

from fullbleed_cli import agent_acceptance, cli


def _contract(path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setattr(cli, "_get_version", lambda: "4.5.6")
    payload = cli._agent_contract_payload()
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def test_acceptance_prepare_is_isolated_and_non_overwriting(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    contract = _contract(tmp_path / "contract.json", monkeypatch)
    workspace = tmp_path / "run"
    result = agent_acceptance.prepare_workspace(
        workspace,
        contract_path=contract,
        scenario_ids=["invoice"],
    )
    assert result["ok"] is True
    scenario = workspace / "invoice"
    assert sorted(path.name for path in scenario.iterdir()) == [
        "FULLBLEED_AGENT_CONTRACT.json",
        "TASK.json",
        "output",
    ]
    task = json.loads((scenario / "TASK.json").read_text(encoding="utf-8"))
    assert task["schema"] == "fullbleed.agent_acceptance_task.v1"
    assert task["scenario"]["id"] == "invoice"

    with pytest.raises(ValueError, match="refusing to overwrite"):
        agent_acceptance.prepare_workspace(
            workspace,
            contract_path=contract,
            scenario_ids=["invoice"],
        )


def test_acceptance_judge_checks_pdf_markers_and_pages(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    contract = _contract(tmp_path / "contract.json", monkeypatch)
    workspace = tmp_path / "run"
    agent_acceptance.prepare_workspace(
        workspace,
        contract_path=contract,
        scenario_ids=["invoice"],
    )
    output = workspace / "invoice" / "output" / "invoice.pdf"
    output.write_bytes(b"%PDF-1.7\nfixture")
    monkeypatch.setattr(
        agent_acceptance.fullbleed,
        "inspect_pdf",
        lambda _path: {
            "page_count": 1,
            "profile": {"claims": []},
            "composition": {"supported": True, "issues": []},
        },
    )
    monkeypatch.setattr(
        agent_acceptance.fullbleed,
        "extract_pdf_page_texts",
        lambda _path: {
            "schema": "fullbleed.pdf.page_text_extract.v1",
            "pages": [
                {"text": "Invoice FB-1042 for Jordan Lee total USD 1,284.50"}
            ],
        },
    )
    result = agent_acceptance.verify_workspace(workspace)
    assert result["ok"] is True
    assert result["metrics"] == {"total": 1, "passed": 1, "failed": 0}

    monkeypatch.setattr(
        agent_acceptance.fullbleed,
        "extract_pdf_page_texts",
        lambda _path: {
            "schema": "fullbleed.pdf.page_text_extract.v1",
            "pages": [{"text": "missing the required values"}],
        },
    )
    failed = agent_acceptance.verify_workspace(workspace)
    assert failed["ok"] is False
    assert {
        failure["code"] for failure in failed["scenarios"][0]["failures"]
    } == {"TEXT_MARKER_MISSING"}
