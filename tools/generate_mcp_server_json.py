#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate MCP Registry server.json from fullbleed-mcp package metadata."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
import tempfile
import os


REPO_ROOT = Path(__file__).resolve().parents[1]
PACKAGE_ROOT = REPO_ROOT / "packages" / "fullbleed-mcp"
PYPROJECT = PACKAGE_ROOT / "pyproject.toml"
DEFAULT_OUT = PACKAGE_ROOT / "server.json"


def _section(source: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^\[{re.escape(name)}\]\s*\n(?P<body>.*?)(?=^\[|\Z)",
        source,
    )
    if not match:
        raise ValueError(f"missing [{name}] in {PYPROJECT}")
    return match.group("body")


def _string(section: str, key: str) -> str:
    match = re.search(
        rf"(?m)^{re.escape(key)}\s*=\s*(?P<value>\"(?:[^\"\\]|\\.)*\")\s*$",
        section,
    )
    if not match:
        raise ValueError(f"missing string {key!r} in package metadata")
    return json.loads(match.group("value"))


def build_server_json(pyproject: Path = PYPROJECT) -> dict:
    source = pyproject.read_text(encoding="utf-8")
    project = _section(source, "project")
    registry = _section(source, "tool.fullbleed-mcp.registry")
    package_name = _string(project, "name")
    version = _string(project, "version")
    description = _string(registry, "description")
    if len(description) > 100:
        raise ValueError("MCP Registry description exceeds the 100-character schema limit")
    return {
        "$schema": _string(registry, "schema"),
        "name": _string(registry, "name"),
        "title": _string(registry, "title"),
        "description": description,
        "version": version,
        "websiteUrl": _string(registry, "website-url"),
        "repository": {
            "url": _string(registry, "repository-url"),
            "source": "github",
        },
        "packages": [
            {
                "registryType": "pypi",
                "identifier": package_name,
                "version": version,
                "transport": {"type": "stdio"},
            }
        ],
    }


def _render(payload: dict) -> str:
    return json.dumps(payload, indent=2, ensure_ascii=True) + "\n"


def _write_atomic(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        newline="\n",
        dir=path.parent,
        prefix=f".{path.name}.",
        suffix=".tmp",
        delete=False,
    ) as handle:
        handle.write(content)
        temporary = Path(handle.name)
    os.replace(temporary, path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        payload = build_server_json()
        expected = _render(payload)
        current = (
            arguments.out.read_text(encoding="utf-8").replace("\r\n", "\n")
            if arguments.out.is_file()
            else None
        )
        current_matches = current == expected
        if not arguments.check:
            _write_atomic(arguments.out, expected)
            current_matches = True
        result = {
            "schema": "fullbleed.mcp_registry_generation.v1",
            "ok": current_matches,
            "mode": "check" if arguments.check else "write",
            "path": str(arguments.out),
            "server": payload["name"],
            "version": payload["version"],
        }
        if arguments.json:
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(
                f"[{'ok' if current_matches else 'stale'}] {payload['name']} "
                f"{payload['version']} server.json\n"
            )
        return 0 if current_matches else 1
    except Exception as exc:
        if arguments.json:
            sys.stdout.write(
                json.dumps(
                    {
                        "schema": "fullbleed.mcp_registry_generation.v1",
                        "ok": False,
                        "code": "MCP_METADATA_GENERATION_FAILED",
                        "message": str(exc),
                    },
                    ensure_ascii=True,
                )
                + "\n"
            )
        else:
            sys.stderr.write(f"[error] {exc}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
