# SPDX-License-Identifier: MIT
"""Agent-agnostic Fullbleed acceptance-suite preparation and judging."""

from __future__ import annotations

import argparse
from importlib import resources
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Mapping

import fullbleed


PREPARE_SCHEMA = "fullbleed.agent_acceptance_prepare.v1"
RESULT_SCHEMA = "fullbleed.agent_acceptance_result.v1"
TASK_SCHEMA = "fullbleed.agent_acceptance_task.v1"
CONTRACT_FILENAME = "FULLBLEED_AGENT_CONTRACT.json"
TASK_FILENAME = "TASK.json"


def _runtime_version() -> str:
    from .cli import _get_version

    return _get_version()


def _load_contract(path: Path | None) -> dict[str, Any]:
    if path is not None:
        raw = path.read_text(encoding="utf-8")
    else:
        packaged = resources.files("fullbleed").joinpath("agent_contract.json")
        if packaged.is_file():
            raw = packaged.read_text(encoding="utf-8")
        else:
            repository = Path(__file__).resolve().parents[2] / "fullbleed-agent-contract.json"
            if repository.is_file():
                raw = repository.read_text(encoding="utf-8")
            else:
                from .cli import _agent_contract_payload

                return _agent_contract_payload()
    parsed = json.loads(raw)
    if not isinstance(parsed, dict) or parsed.get("schema") != "fullbleed.agent_contract.v1":
        raise ValueError("contract is not a fullbleed.agent_contract.v1 object")
    return parsed


def _scenario_map(contract: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    raw = contract.get("acceptance_suite", {}).get("scenarios", [])
    result = {}
    for scenario in raw:
        if isinstance(scenario, dict) and isinstance(scenario.get("id"), str):
            result[scenario["id"]] = scenario
    if len(result) < 5:
        raise ValueError("agent contract does not contain the complete acceptance suite")
    return result


def _selected_scenarios(
    contract: Mapping[str, Any], scenario_ids: list[str] | None
) -> list[dict[str, Any]]:
    available = _scenario_map(contract)
    if not scenario_ids:
        return [available[key] for key in available]
    unknown = sorted(set(scenario_ids) - set(available))
    if unknown:
        raise ValueError(f"unknown acceptance scenario(s): {', '.join(unknown)}")
    return [available[key] for key in scenario_ids]


def _assert_blank_workspace(workspace: Path) -> Path:
    resolved = workspace.resolve()
    if resolved.exists():
        if not resolved.is_dir():
            raise ValueError(f"acceptance workspace is not a directory: {resolved}")
        if any(resolved.iterdir()):
            raise ValueError(
                f"acceptance workspace must be absent or empty; refusing to overwrite {resolved}"
            )
    else:
        resolved.mkdir(parents=True)
    return resolved


def _write_template_fixture(path: Path) -> None:
    engine = fullbleed.PdfEngine(
        document_lang="en-US",
        document_title="Agent Acceptance Form Template",
    )
    html = (
        "<!doctype html><html lang='en'><head><title>Form template</title></head>"
        "<body><main><h1>Enrollment Form</h1><p>FORM-TEMPLATE-001</p>"
        "<div class='box'>Account holder</div><div class='box'>Authorization</div>"
        "</main></body></html>"
    )
    css = (
        "@page{size:letter;margin:0.6in}body{font-family:Helvetica,sans-serif;color:#172033}"
        "h1{font-size:20pt;border-bottom:2pt solid #304a77;padding-bottom:8pt}"
        ".box{height:120pt;border:1pt solid #65708a;margin:20pt 0;padding:10pt;color:#65708a}"
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(engine.render_pdf(html, css))


def prepare_workspace(
    workspace: Path,
    *,
    contract_path: Path | None = None,
    scenario_ids: list[str] | None = None,
) -> dict[str, Any]:
    """Create isolated, non-overwriting scenario directories."""
    contract = _load_contract(contract_path)
    root = _assert_blank_workspace(workspace)
    scenarios = _selected_scenarios(contract, scenario_ids)
    contract_text = json.dumps(contract, indent=2, sort_keys=True, ensure_ascii=True) + "\n"
    prepared = []
    for scenario in scenarios:
        scenario_root = root / scenario["id"]
        scenario_root.mkdir()
        (scenario_root / "output").mkdir()
        (scenario_root / CONTRACT_FILENAME).write_text(contract_text, encoding="utf-8")
        task = {
            "schema": TASK_SCHEMA,
            "scenario": scenario,
            "rules": [
                f"Treat {CONTRACT_FILENAME} as the only Fullbleed product documentation.",
                "Do not request human correction or use a browser PDF renderer.",
                "Place every deliverable under the output directory exactly as requested.",
                "Inspect and verify the final PDF before declaring the task complete.",
            ],
        }
        (scenario_root / TASK_FILENAME).write_text(
            json.dumps(task, indent=2, sort_keys=True, ensure_ascii=True) + "\n",
            encoding="utf-8",
        )
        if scenario.get("fixture") == "pdf_template":
            _write_template_fixture(scenario_root / "inputs" / "form-template.pdf")
        prepared.append(
            {
                "id": scenario["id"],
                "path": str(scenario_root),
                "task": str(scenario_root / TASK_FILENAME),
                "contract": str(scenario_root / CONTRACT_FILENAME),
            }
        )
    return {
        "schema": PREPARE_SCHEMA,
        "ok": True,
        "runtime_version": _runtime_version(),
        "contract_version": contract.get("product", {}).get("version"),
        "workspace": str(root),
        "scenarios": prepared,
    }


def _safe_child(root: Path, relative: str) -> Path:
    candidate = (root / relative).resolve()
    resolved_root = root.resolve(strict=True)
    if candidate != resolved_root and resolved_root not in candidate.parents:
        raise ValueError(f"scenario path escapes workspace: {relative}")
    return candidate


def _pdf_text(path: Path) -> tuple[str, dict[str, Any] | None]:
    extractor = getattr(fullbleed, "extract_pdf_page_texts", None)
    if not callable(extractor):
        return "", None
    report = dict(extractor(str(path)))
    pages = report.get("pages", [])
    text = "\n".join(
        str(page.get("text") or "") for page in pages if isinstance(page, Mapping)
    )
    return text, report


def _check_scenario(root: Path, scenario: Mapping[str, Any]) -> dict[str, Any]:
    failures: list[dict[str, Any]] = []

    def fail(code: str, message: str, **detail: Any) -> None:
        failures.append({"code": code, "message": message, **detail})

    deliverable = _safe_child(root, str(scenario["deliverable"]))
    if not deliverable.is_file():
        fail("DELIVERABLE_MISSING", f"missing PDF deliverable: {scenario['deliverable']}")
        return {
            "id": scenario["id"],
            "ok": False,
            "deliverable": str(deliverable),
            "failures": failures,
        }
    try:
        inspection = dict(fullbleed.inspect_pdf(str(deliverable)))
    except Exception as exc:
        fail("PDF_INSPECT_FAILED", str(exc))
        inspection = {}
    checks = scenario.get("checks", {})
    page_count = inspection.get("page_count")
    if not isinstance(page_count, int):
        fail("PAGE_COUNT_MISSING", "PDF inspection did not return an integer page count")
    else:
        minimum = checks.get("min_pages")
        maximum = checks.get("max_pages")
        if isinstance(minimum, int) and page_count < minimum:
            fail("PAGE_COUNT_LOW", f"expected at least {minimum} page(s), observed {page_count}")
        if isinstance(maximum, int) and page_count > maximum:
            fail("PAGE_COUNT_HIGH", f"expected at most {maximum} page(s), observed {page_count}")
    try:
        text, extraction = _pdf_text(deliverable)
    except Exception as exc:
        fail("PDF_TEXT_EXTRACTION_FAILED", str(exc))
        text, extraction = "", None
    for marker in checks.get("text_markers", []):
        if marker not in text:
            fail("TEXT_MARKER_MISSING", f"PDF text is missing marker {marker!r}", marker=marker)
    cursor = -1
    for marker in checks.get("ordered_text_markers", []):
        position = text.find(marker, cursor + 1)
        if position < 0:
            fail("TEXT_ORDER_INVALID", f"ordered marker {marker!r} is absent or out of order")
            break
        cursor = position
    profile = inspection.get("profile") if isinstance(inspection.get("profile"), dict) else {}
    expected_claims = set(checks.get("any_profile_claims", []))
    if expected_claims and not expected_claims.intersection(profile.get("claims", [])):
        fail(
            "PDF_PROFILE_MISSING",
            f"expected one of {sorted(expected_claims)}, observed {profile.get('claims', [])}",
        )
    for key in checks.get("profile_truthy", []):
        if not profile.get(key):
            fail("PDF_PROFILE_FIELD_FALSE", f"expected profile.{key} to be true", field=key)
    for key in checks.get("profile_empty", []):
        if profile.get(key):
            fail("PDF_PROFILE_FIELD_NONEMPTY", f"expected profile.{key} to be empty", field=key)
    if checks.get("composition_supported") is True:
        composition = inspection.get("composition") or {}
        if composition.get("supported") is not True:
            fail(
                "PDF_COMPOSITION_UNSUPPORTED",
                "final PDF is not reported composition-compatible",
                issues=composition.get("issues", []),
            )
    evidence_contract = checks.get("evidence")
    evidence = None
    if isinstance(evidence_contract, Mapping):
        evidence_path = _safe_child(root, str(evidence_contract["path"]))
        try:
            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
        except Exception as exc:
            fail("EVIDENCE_INVALID", f"unable to read compiled-lane evidence: {exc}")
        else:
            if not isinstance(evidence, Mapping):
                fail("EVIDENCE_INVALID", "compiled-lane evidence must be a JSON object")
            else:
                if evidence.get("schema") != evidence_contract.get("schema"):
                    fail("EVIDENCE_SCHEMA_INVALID", "compiled-lane evidence schema is invalid")
                if evidence.get("record_count") != evidence_contract.get("record_count"):
                    fail("EVIDENCE_RECORD_COUNT_INVALID", "compiled-lane record count is invalid")
                if evidence.get("api") not in evidence_contract.get("allowed_apis", []):
                    fail("EVIDENCE_API_INVALID", "evidence does not name an allowed compiled API")
    return {
        "id": scenario["id"],
        "ok": not failures,
        "deliverable": str(deliverable),
        "bytes": deliverable.stat().st_size,
        "page_count": page_count,
        "inspection": inspection,
        "text_extraction_schema": extraction.get("schema") if extraction else None,
        "evidence": evidence,
        "failures": failures,
    }


def verify_workspace(
    workspace: Path,
    *,
    scenario_ids: list[str] | None = None,
) -> dict[str, Any]:
    root = workspace.resolve(strict=True)
    if not root.is_dir():
        raise ValueError(f"acceptance workspace is not a directory: {root}")
    available_dirs = {child.name: child for child in root.iterdir() if child.is_dir()}
    if scenario_ids:
        selected_ids = scenario_ids
    else:
        selected_ids = sorted(available_dirs)
    results = []
    for scenario_id in selected_ids:
        scenario_root = available_dirs.get(scenario_id)
        if scenario_root is None:
            results.append(
                {
                    "id": scenario_id,
                    "ok": False,
                    "failures": [
                        {
                            "code": "SCENARIO_DIRECTORY_MISSING",
                            "message": f"missing scenario directory: {scenario_id}",
                        }
                    ],
                }
            )
            continue
        task_path = scenario_root / TASK_FILENAME
        try:
            task = json.loads(task_path.read_text(encoding="utf-8"))
            scenario = task["scenario"]
        except Exception as exc:
            results.append(
                {
                    "id": scenario_id,
                    "ok": False,
                    "failures": [
                        {
                            "code": "TASK_INVALID",
                            "message": f"unable to load task contract: {exc}",
                        }
                    ],
                }
            )
            continue
        results.append(_check_scenario(scenario_root, scenario))
    passed = sum(1 for result in results if result["ok"])
    return {
        "schema": RESULT_SCHEMA,
        "ok": passed == len(results) and bool(results),
        "runtime_version": _runtime_version(),
        "workspace": str(root),
        "scenarios": results,
        "metrics": {
            "total": len(results),
            "passed": passed,
            "failed": len(results) - passed,
        },
    }


def run_agents(
    workspace: Path,
    *,
    agent_command: list[str],
    contract_path: Path | None = None,
    scenario_ids: list[str] | None = None,
    timeout_seconds: int = 900,
) -> dict[str, Any]:
    if not agent_command:
        raise ValueError("agent command is required")
    prepared = prepare_workspace(
        workspace,
        contract_path=contract_path,
        scenario_ids=scenario_ids,
    )
    agent_runs = []
    for item in prepared["scenarios"]:
        scenario_root = Path(item["path"])
        substitutions = {
            "workspace": str(scenario_root),
            "task": item["task"],
            "contract": item["contract"],
            "scenario": item["id"],
        }
        command = [part.format(**substitutions) for part in agent_command]
        environment = os.environ.copy()
        environment.update(
            {
                "FULLBLEED_ACCEPTANCE_WORKSPACE": str(scenario_root),
                "FULLBLEED_ACCEPTANCE_TASK": item["task"],
                "FULLBLEED_AGENT_CONTRACT": item["contract"],
                "FULLBLEED_ACCEPTANCE_SCENARIO": item["id"],
            }
        )
        try:
            completed = subprocess.run(
                command,
                cwd=scenario_root,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                timeout=timeout_seconds,
                check=False,
            )
            exit_code = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except subprocess.TimeoutExpired as exc:
            exit_code = None
            stdout = exc.stdout or ""
            stderr = (exc.stderr or "") + f"\nagent timed out after {timeout_seconds}s"
        (scenario_root / "agent.stdout.log").write_text(stdout, encoding="utf-8")
        (scenario_root / "agent.stderr.log").write_text(stderr, encoding="utf-8")
        agent_runs.append(
            {
                "id": item["id"],
                "command": command,
                "exit_code": exit_code,
                "timed_out": exit_code is None,
            }
        )
    report = verify_workspace(workspace, scenario_ids=scenario_ids)
    report["agent_runs"] = agent_runs
    if any(run["exit_code"] != 0 for run in agent_runs):
        report["ok"] = False
    return report


def _emit(payload: Mapping[str, Any], *, json_output: bool) -> None:
    if json_output:
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
        return
    metrics = payload.get("metrics")
    if isinstance(metrics, Mapping):
        sys.stdout.write(
            f"[{'ok' if payload.get('ok') else 'fail'}] agent acceptance: "
            f"{metrics.get('passed', 0)}/{metrics.get('total', 0)} passed\n"
        )
    else:
        sys.stdout.write(
            f"[ok] prepared {len(payload.get('scenarios', []))} scenario(s) at "
            f"{payload.get('workspace')}\n"
        )


def cmd_prepare(args: argparse.Namespace) -> None:
    payload = prepare_workspace(
        Path(args.workspace),
        contract_path=Path(args.contract) if args.contract else None,
        scenario_ids=args.scenario,
    )
    _emit(payload, json_output=bool(args.json))


def cmd_verify(args: argparse.Namespace) -> None:
    payload = verify_workspace(Path(args.workspace), scenario_ids=args.scenario)
    _emit(payload, json_output=bool(args.json))
    if not payload["ok"]:
        raise SystemExit(1)


def cmd_run(args: argparse.Namespace) -> None:
    command = list(args.agent_command or [])
    if command and command[0] == "--":
        command = command[1:]
    payload = run_agents(
        Path(args.workspace),
        agent_command=command,
        contract_path=Path(args.contract) if args.contract else None,
        scenario_ids=args.scenario,
        timeout_seconds=args.timeout,
    )
    _emit(payload, json_output=bool(args.json))
    if not payload["ok"]:
        raise SystemExit(1)


def add_cli_parser(subparsers: Any) -> None:
    """Register acceptance commands on the Fullbleed CLI parser."""
    parser = subparsers.add_parser(
        "agent-acceptance",
        help="Prepare, run, and judge isolated agent acceptance scenarios",
    )
    actions = parser.add_subparsers(dest="agent_acceptance_command", required=True)
    for name, handler, help_text in (
        ("prepare", cmd_prepare, "Prepare blank scenario workspaces"),
        ("verify", cmd_verify, "Judge existing scenario deliverables"),
    ):
        action = actions.add_parser(name, help=help_text)
        action.add_argument("--workspace", required=True)
        action.add_argument("--scenario", action="append")
        if name == "prepare":
            action.add_argument("--contract")
        action.add_argument("--json", action="store_true")
        action.set_defaults(func=handler)
    run = actions.add_parser("run", help="Prepare, invoke an external agent, and judge")
    run.add_argument("--workspace", required=True)
    run.add_argument("--scenario", action="append")
    run.add_argument("--contract")
    run.add_argument("--timeout", type=int, default=900)
    run.add_argument(
        "--agent-command",
        nargs=argparse.REMAINDER,
        required=True,
        help="External command; supports {workspace}, {task}, {contract}, and {scenario}",
    )
    run.add_argument("--json", action="store_true")
    run.set_defaults(func=cmd_run)


__all__ = [
    "add_cli_parser",
    "prepare_workspace",
    "run_agents",
    "verify_workspace",
]
