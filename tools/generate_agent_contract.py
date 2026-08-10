#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate agent-facing artifacts from an installed Fullbleed binary."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = REPO_ROOT / "fullbleed-agent-contract.json"
DEFAULT_MARKDOWN = REPO_ROOT / "cli_schema.md"
DEFAULT_LLMS = REPO_ROOT / "llms.txt"
EXPECTED_SCHEMA = "fullbleed.agent_contract.v1"
REQUIRED_ENGINE_FIELDS = {
    "compiled_document",
    "compiled_reflow_bindings",
    "compiled_flow_compression_modes",
    "batch_render",
    "batch_render_parallel",
}


def _run_runtime(python: str, output_format: str, cwd: Path) -> str:
    command = [
        python,
        "-m",
        "fullbleed",
        "agent-contract",
        "--format",
        output_format,
    ]
    environment = os.environ.copy()
    environment["PYTHONIOENCODING"] = "utf-8"
    environment["PYTHONUTF8"] = "1"
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        check=False,
        timeout=120,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(
            f"installed Fullbleed runtime failed ({completed.returncode}): {detail}"
        )
    return completed.stdout.replace("\r\n", "\n")


def _validate_contract(contract: Any, *, allow_dev_version: bool) -> dict[str, Any]:
    if not isinstance(contract, dict):
        raise ValueError("agent contract must be a JSON object")
    if contract.get("schema") != EXPECTED_SCHEMA:
        raise ValueError(
            f"expected schema {EXPECTED_SCHEMA!r}, observed {contract.get('schema')!r}"
        )
    product = contract.get("product")
    if not isinstance(product, dict) or not isinstance(product.get("version"), str):
        raise ValueError("agent contract is missing product.version")
    if not allow_dev_version and product["version"] == "0.0.0-dev":
        raise ValueError("refusing to generate release artifacts from a development fallback version")
    capabilities = contract.get("capabilities")
    if not isinstance(capabilities, dict):
        raise ValueError("agent contract is missing capabilities")
    engine = capabilities.get("engine")
    if not isinstance(engine, dict):
        raise ValueError("agent contract is missing capabilities.engine")
    missing_engine = sorted(REQUIRED_ENGINE_FIELDS - set(engine))
    if missing_engine:
        raise ValueError(f"capabilities.engine is missing: {', '.join(missing_engine)}")
    commands = contract.get("commands")
    if not isinstance(commands, dict) or not isinstance(commands.get("surface"), dict):
        raise ValueError("agent contract is missing parser-derived commands.surface")
    if set(commands["surface"]) != set(capabilities.get("commands", [])):
        raise ValueError("capabilities.commands disagrees with parser-derived commands.surface")
    schemas = contract.get("schemas", {}).get("definitions", {})
    registry = commands.get("schema_registry", {})
    missing_schemas = sorted(set(registry.values()) - set(schemas))
    if missing_schemas:
        raise ValueError(f"schema definitions are missing: {', '.join(missing_schemas)}")
    if not contract.get("recommendation_boundary"):
        raise ValueError("agent contract is missing recommendation_boundary")
    if not contract.get("tool_adapter", {}).get("tools"):
        raise ValueError("agent contract is missing tool_adapter.tools")
    if len(contract.get("acceptance_suite", {}).get("scenarios", [])) < 5:
        raise ValueError("agent contract must contain the five first-party acceptance scenarios")
    return contract


def generate(
    *,
    python: str,
    allow_dev_version: bool = False,
) -> tuple[str, str, str, dict[str, Any]]:
    """Read both artifacts from an installed runtime in an isolated working directory."""
    with tempfile.TemporaryDirectory(prefix="fullbleed-agent-contract-") as raw:
        runtime_cwd = Path(raw)
        json_text = _run_runtime(python, "json", runtime_cwd)
        markdown = _run_runtime(python, "markdown", runtime_cwd)
        llms = _run_runtime(python, "llms", runtime_cwd)
    try:
        parsed = json.loads(json_text)
    except json.JSONDecodeError as exc:
        raise ValueError(f"installed runtime returned invalid contract JSON: {exc}") from exc
    contract = _validate_contract(parsed, allow_dev_version=allow_dev_version)
    canonical_json = json.dumps(
        contract,
        indent=2,
        sort_keys=True,
        ensure_ascii=True,
    ) + "\n"
    if not markdown.endswith("\n"):
        markdown += "\n"
    expected_version = contract["product"]["version"]
    if f"Fullbleed **{expected_version}**" not in markdown:
        raise ValueError("runtime Markdown does not identify the same product version")
    if "compiled_reflow_bindings" not in markdown:
        raise ValueError("runtime Markdown omits compiled reflow capabilities")
    if not llms.endswith("\n"):
        llms += "\n"
    if f"Fullbleed {expected_version}" not in llms:
        raise ValueError("runtime llms.txt does not identify the same product version")
    if "fullbleed agent-contract --format json" not in llms:
        raise ValueError("runtime llms.txt omits authoritative discovery")
    return canonical_json, markdown, llms, contract


def _atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        newline="\n",
        prefix=f".{path.name}.",
        suffix=".tmp",
        dir=path.parent,
        delete=False,
    ) as handle:
        handle.write(content)
        temporary = Path(handle.name)
    os.replace(temporary, path)


def _check(path: Path, expected: str) -> bool:
    try:
        observed = path.read_text(encoding="utf-8").replace("\r\n", "\n")
    except FileNotFoundError:
        return False
    return observed == expected


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate canonical agent artifacts from an installed Fullbleed binary."
    )
    parser.add_argument(
        "--python",
        default=sys.executable,
        help="Python interpreter containing the built Fullbleed wheel",
    )
    parser.add_argument("--contract-out", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--markdown-out", type=Path, default=DEFAULT_MARKDOWN)
    parser.add_argument("--llms-out", type=Path, default=DEFAULT_LLMS)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Fail if committed artifacts differ; do not write files",
    )
    parser.add_argument(
        "--allow-dev-version",
        action="store_true",
        help="Permit the 0.0.0-dev metadata fallback for local experiments",
    )
    parser.add_argument("--json", action="store_true", help="Emit a JSON status result")
    arguments = parser.parse_args(argv)

    try:
        json_text, markdown, llms, contract = generate(
            python=arguments.python,
            allow_dev_version=arguments.allow_dev_version,
        )
        checks = {
            str(arguments.contract_out): _check(arguments.contract_out, json_text),
            str(arguments.markdown_out): _check(arguments.markdown_out, markdown),
            str(arguments.llms_out): _check(arguments.llms_out, llms),
        }
        if arguments.check:
            ok = all(checks.values())
        else:
            _atomic_write(arguments.contract_out, json_text)
            _atomic_write(arguments.markdown_out, markdown)
            _atomic_write(arguments.llms_out, llms)
            checks = {key: True for key in checks}
            ok = True
        result = {
            "schema": "fullbleed.agent_contract_generation.v1",
            "ok": ok,
            "mode": "check" if arguments.check else "write",
            "runtime_version": contract["product"]["version"],
            "artifacts": checks,
        }
        if arguments.json:
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            status = "ok" if ok else "stale"
            sys.stdout.write(
                f"[{status}] Fullbleed {result['runtime_version']} agent artifacts ({result['mode']})\n"
            )
            for path, current in checks.items():
                sys.stdout.write(f"- {path}: {'current' if current else 'stale or missing'}\n")
        return 0 if ok else 1
    except Exception as exc:
        result = {
            "schema": "fullbleed.agent_contract_generation.v1",
            "ok": False,
            "code": "AGENT_CONTRACT_GENERATION_FAILED",
            "message": str(exc),
        }
        if arguments.json:
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] {exc}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
