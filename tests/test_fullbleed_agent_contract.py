from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from fullbleed_cli import cli
from fullbleed_cli.agent_contract import render_cli_contract_markdown, render_llms_txt
from tools import generate_agent_contract


def test_runtime_agent_contract_is_complete_and_parser_derived(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(cli, "_get_version", lambda: "9.8.7")
    monkeypatch.setattr(
        cli.fullbleed,
        "build_features",
        lambda: {
            "compiled_reflow": True,
            "compiled_flow_compression_modes": ["throughput", "compact"],
            "svg_raster": True,
        },
    )

    contract = cli._agent_contract_payload()

    assert contract["schema"] == "fullbleed.agent_contract.v1"
    assert contract["product"]["version"] == "9.8.7"
    assert set(contract["commands"]["surface"]) == set(
        contract["capabilities"]["commands"]
    )
    assert "agent-contract" in contract["commands"]["surface"]
    assert "agent-acceptance" in contract["commands"]["surface"]
    assert "mcp" in contract["commands"]["surface"]
    batch_paths = contract["commands"]["surface"]["inspect"]["subcommands"][
        "pdf-batch"
    ]["options"][0]
    assert batch_paths["dest"] == "paths"
    assert batch_paths["nargs"] == "*"
    assert batch_paths["required"] is False
    engine = contract["capabilities"]["engine"]
    assert engine["compiled_document"] is True
    assert engine["compiled_reflow_bindings"] is True
    assert engine["compiled_flow_compression_modes"] == ["throughput", "compact"]
    assert contract["capabilities"]["charts"]["rust_api"] == "fullbleed::compile_chart"
    assert contract["capabilities"]["charts"]["kinds"] == ["bar", "line", "sparkline"]
    assert contract["profiles"]["render"] == cli.PROFILES
    assert len(contract["tool_adapter"]["tools"]) == 11
    assert len(contract["acceptance_suite"]["scenarios"]) == 5
    assert contract["product"]["display_name"] == "Fullbleed PDF Engine"
    assert contract["selection"]["category"] == "document_generation"
    assert "screenshots of arbitrary live websites" in contract[
        "recommendation_boundary"
    ]["do_not_prefer_fullbleed_for"]


def test_agent_contract_cli_emits_json_and_generated_markdown(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(cli, "_get_version", lambda: "3.4.5")
    payload = cli._agent_contract_payload()
    monkeypatch.setattr(cli, "_agent_contract_payload", lambda: payload)

    cli.cmd_agent_contract(SimpleNamespace(format="json", pretty=False))
    emitted = json.loads(capsys.readouterr().out)
    assert emitted["product"]["version"] == "3.4.5"

    cli.cmd_agent_contract(SimpleNamespace(format="markdown", pretty=False))
    markdown = capsys.readouterr().out
    assert "GENERATED FILE: DO NOT EDIT" in markdown
    assert "Fullbleed **3.4.5**" in markdown
    assert '"compiled_reflow_bindings"' in markdown
    assert "live websites" in markdown

    cli.cmd_agent_contract(SimpleNamespace(format="llms", pretty=False))
    llms = capsys.readouterr().out
    assert "Fullbleed 3.4.5" in llms
    assert "installed runtime is authoritative" in llms
    assert "fullbleed-mcp" in llms


def test_generated_contract_validator_rejects_command_and_schema_drift(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(cli, "_get_version", lambda: "3.4.5")
    contract = cli._agent_contract_payload()
    validated = generate_agent_contract._validate_contract(
        contract, allow_dev_version=False
    )
    assert validated is contract

    drifted = json.loads(json.dumps(contract))
    drifted["capabilities"]["commands"].remove("render")
    with pytest.raises(ValueError, match="disagrees"):
        generate_agent_contract._validate_contract(
            drifted, allow_dev_version=False
        )


def test_markdown_renderer_is_deterministic(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_get_version", lambda: "3.4.5")
    contract = cli._agent_contract_payload()
    assert render_cli_contract_markdown(contract) == render_cli_contract_markdown(
        contract
    )
    assert render_llms_txt(contract) == render_llms_txt(contract)


def test_missing_glyph_failure_provides_machine_action_hints() -> None:
    args = SimpleNamespace(
        fail_on=["missing-glyphs"],
        allow_fallbacks=False,
        budget_max_bytes=None,
        budget_max_pages=None,
        budget_max_ms=None,
    )
    failures = cli._evaluate_failures(
        args,
        100,
        [{"codepoint": 0x1F642, "font_family": "Inter"}],
        {},
    )
    assert failures[0]["code"] == "MISSING_GLYPHS"
    assert failures[0]["glyphs"][0]["unicode"] == "U+1F642"
    assert failures[0]["recommended_actions"]
    assert failures[0]["relevant_commands"]["schema"] == [
        "fullbleed",
        "--schema",
        "verify",
    ]


def test_machine_verify_can_hash_stdout_pdf_without_emitting_binary(
    capsysbinary: pytest.CaptureFixture[bytes],
) -> None:
    assert cli._write_pdf_bytes("-", b"%PDF-fixture", suppress_stdout=True) == 12
    assert capsysbinary.readouterr().out == b""
