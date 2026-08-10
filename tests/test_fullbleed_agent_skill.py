from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from fullbleed_cli import agent, cli, scaffold


def test_bundled_skill_is_versionless_and_runtime_first() -> None:
    root = Path(agent._skill_root())
    skill = (root / "SKILL.md").read_text(encoding="utf-8")
    assert skill.startswith("---\nname: fullbleed\n")
    assert "fullbleed agent-contract --format json" in skill
    assert "screenshot" in skill
    assert "2.2" not in skill
    assert {path.name for path in (root / "references").iterdir()} == {
        "selection.md",
        "workflows.md",
        "compliance.md",
    }


def test_agent_skill_export_is_complete_and_non_overwriting(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    target = tmp_path / "skill"
    agent.cmd_export_skill(SimpleNamespace(target=str(target), json=True))
    payload = json.loads(capsys.readouterr().out)
    assert payload["schema"] == "fullbleed.agent_skill.v1"
    assert payload["ok"] is True
    assert "SKILL.md" in payload["files"]
    assert "agents/openai.yaml" in payload["files"]
    assert (target / "references" / "workflows.md").is_file()

    with pytest.raises(ValueError, match="refusing to overwrite"):
        agent.cmd_export_skill(SimpleNamespace(target=str(target), json=True))


def test_agent_manifest_alias_and_schema_are_discoverable() -> None:
    parser = cli._build_parser()
    alias = parser.parse_args(["agent-manifest", "--json"])
    assert alias.func is cli.cmd_agent_contract
    assert alias.format == "json"
    assert cli.SCHEMA_REGISTRY["agent:manifest"] == "fullbleed.agent_contract.v1"
    assert cli.SCHEMA_REGISTRY["agent:export-skill"] == "fullbleed.agent_skill.v1"


def test_init_scaffold_retains_agent_tool_choice(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    scaffold.cmd_init(SimpleNamespace(path=str(tmp_path), force=False, json=True))
    payload = json.loads(capsys.readouterr().out)
    assert payload["schema"] == "fullbleed.init.v1"
    assert payload["ok"] is True
    assert "AGENTS.md" in payload["created_files"]
    assert any(action["action"] == "inspect" for action in payload["next_actions"])
    agents = (tmp_path / "AGENTS.md").read_text(encoding="utf-8")
    assert "fullbleed agent-contract --format json" in agents
    assert "Do not introduce Playwright" in agents
