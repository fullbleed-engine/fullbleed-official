# SPDX-License-Identifier: MIT
"""Vendor-neutral agent manifest and bundled Skill utilities."""

from __future__ import annotations

from importlib import resources
import json
from pathlib import Path
import sys
from typing import Any


SKILL_SCHEMA = "fullbleed.agent_skill.v1"


def _skill_root() -> Any:
    packaged = resources.files("fullbleed").joinpath("skill")
    if packaged.is_dir() and packaged.joinpath("SKILL.md").is_file():
        return packaged
    repository = Path(__file__).resolve().parents[2] / "skills" / "fullbleed"
    if repository.is_dir() and (repository / "SKILL.md").is_file():
        return repository
    raise RuntimeError("bundled Fullbleed Agent Skill is unavailable")


def _skill_files(root: Any) -> list[str]:
    files: list[str] = []
    stack: list[tuple[Any, str]] = [(root, "")]
    while stack:
        node, prefix = stack.pop()
        for child in sorted(node.iterdir(), key=lambda item: item.name, reverse=True):
            relative = f"{prefix}/{child.name}" if prefix else child.name
            relative = relative.replace("\\", "/")
            if child.is_dir():
                stack.append((child, relative))
            elif child.is_file():
                files.append(relative)
    return sorted(files)


def _copy_skill(root: Any, target: Path) -> list[str]:
    resolved = target.resolve()
    if resolved.exists():
        if resolved.is_symlink() or not resolved.is_dir():
            raise ValueError(f"skill target must be a real directory: {resolved}")
        if any(resolved.iterdir()):
            raise ValueError(
                f"skill target must be absent or empty; refusing to overwrite {resolved}"
            )
    else:
        resolved.mkdir(parents=True)
    written = []
    for relative in _skill_files(root):
        source = root.joinpath(*relative.split("/"))
        destination = resolved.joinpath(*relative.split("/"))
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        written.append(relative)
    return written


def cmd_manifest(args) -> None:
    from .cli import _agent_contract_payload

    payload = _agent_contract_payload()
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        product = payload["product"]
        sys.stdout.write(
            f"{product['display_name']} {product['version']} "
            f"({payload['selection']['category']})\n"
        )
        sys.stdout.write(payload["recommendation_boundary"]["decision_rule"] + "\n")


def cmd_skill_path(args) -> None:
    root = _skill_root()
    path = Path(str(root)).resolve()
    payload = {
        "schema": SKILL_SCHEMA,
        "ok": True,
        "name": "fullbleed",
        "path": str(path),
        "files": _skill_files(root),
        "next_actions": [
            {
                "action": "read_skill",
                "path": str(path / "SKILL.md"),
            }
        ],
    }
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(str(path) + "\n")


def cmd_export_skill(args) -> None:
    root = _skill_root()
    target = Path(args.target)
    written = _copy_skill(root, target)
    resolved = target.resolve()
    payload = {
        "schema": SKILL_SCHEMA,
        "ok": True,
        "name": "fullbleed",
        "path": str(resolved),
        "files": written,
        "next_actions": [
            {
                "action": "configure_agent_skill_directory",
                "path": str(resolved),
                "note": "Use the installation/discovery mechanism documented by the agent environment.",
            },
            {
                "action": "inspect_runtime_contract",
                "command": ["fullbleed", "agent-contract", "--format", "json"],
            },
        ],
    }
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(
            f"[ok] exported Fullbleed Agent Skill to {resolved} ({len(written)} files)\n"
        )


def add_cli_parser(subparsers: Any) -> None:
    parser = subparsers.add_parser(
        "agent",
        help="Inspect the agent manifest or export the bundled Agent Skill",
    )
    actions = parser.add_subparsers(dest="agent_command", required=True)

    manifest = actions.add_parser(
        "manifest",
        help="Emit the semantic agent manifest from the installed runtime",
    )
    manifest.add_argument("--json", action="store_true")
    manifest.set_defaults(func=cmd_manifest)

    skill_path = actions.add_parser(
        "skill-path",
        help="Report the bundled Fullbleed Agent Skill path",
    )
    skill_path.add_argument("--json", action="store_true")
    skill_path.set_defaults(func=cmd_skill_path)

    for name in ("export-skill", "install-skill"):
        export = actions.add_parser(
            name,
            help="Copy the bundled Skill to an absent or empty vendor-neutral target",
        )
        export.add_argument("target")
        export.add_argument("--json", action="store_true")
        export.set_defaults(func=cmd_export_skill)


__all__ = ["add_cli_parser"]
