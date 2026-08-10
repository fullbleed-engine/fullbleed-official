#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prepare and machine-score the initial approach-neutral AgentDocBench tasks."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
from typing import Any, Mapping


REPO_ROOT = Path(__file__).resolve().parents[1]
TASKS_PATH = REPO_ROOT / "agentdocbench" / "tasks.json"
TASK_FILENAME = "TASK.json"


def _tasks() -> dict[str, dict[str, Any]]:
    payload = json.loads(TASKS_PATH.read_text(encoding="utf-8"))
    if payload.get("schema") != "agentdocbench.tasks.v1":
        raise ValueError("AgentDocBench task registry has the wrong schema")
    tasks = {task["id"]: task for task in payload.get("tasks", [])}
    if len(tasks) != len(payload.get("tasks", [])):
        raise ValueError("AgentDocBench task ids must be unique")
    return tasks


def _blank(path: Path) -> Path:
    resolved = path.resolve()
    if resolved.exists():
        if not resolved.is_dir() or any(resolved.iterdir()):
            raise ValueError(f"workspace must be absent or empty: {resolved}")
    else:
        resolved.mkdir(parents=True)
    return resolved


def _template_fixture(path: Path) -> None:
    import fullbleed

    engine = fullbleed.PdfEngine(document_title="AgentDocBench Template")
    pdf = engine.render_pdf(
        "<main><h1>Enrollment Form</h1><p>ADB-TEMPLATE-BASE</p>"
        "<div class='field'>Account holder</div></main>",
        "@page{size:letter;margin:.6in}body{font:12pt Helvetica,sans-serif}"
        ".field{height:180pt;border:1pt solid #777;padding:10pt}",
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(pdf)


def prepare(workspace: Path, task_ids: list[str] | None = None) -> dict[str, Any]:
    registry = _tasks()
    selected = task_ids or list(registry)
    unknown = sorted(set(selected) - set(registry))
    if unknown:
        raise ValueError(f"unknown task(s): {', '.join(unknown)}")
    root = _blank(workspace)
    prepared = []
    for task_id in selected:
        task = registry[task_id]
        task_root = root / task_id
        (task_root / "inputs").mkdir(parents=True)
        (task_root / "output").mkdir()
        (task_root / TASK_FILENAME).write_text(
            json.dumps(
                {
                    "schema": "agentdocbench.task.v1",
                    "task": task,
                    "submission_schema": "agentdocbench.submission.v1",
                },
                indent=2,
                ensure_ascii=False,
            )
            + "\n",
            encoding="utf-8",
        )
        for filename, value in task.get("inputs", {}).items():
            (task_root / "inputs" / filename).write_text(
                json.dumps(value, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
            )
        if task.get("fixture") == "pdf_template":
            _template_fixture(task_root / "inputs" / "form-template.pdf")
        prepared.append({"id": task_id, "path": str(task_root)})
    return {
        "schema": "agentdocbench.prepare_result.v1",
        "ok": True,
        "workspace": str(root),
        "tasks": prepared,
    }


def _text(fullbleed: Any, path: Path) -> str:
    report = fullbleed.extract_pdf_page_texts(str(path))
    return "\n".join(
        str(page.get("text") or "")
        for page in report.get("pages", [])
        if isinstance(page, Mapping)
    )


def _score_task(task_root: Path, task: Mapping[str, Any], fullbleed: Any) -> dict[str, Any]:
    failures = []
    deliverable = task_root / task["deliverable"]
    inspection = {}
    text = ""
    if not deliverable.is_file():
        failures.append({"code": "DELIVERABLE_MISSING", "path": str(deliverable)})
    else:
        try:
            inspection = dict(fullbleed.inspect_pdf(str(deliverable)))
            text = _text(fullbleed, deliverable)
        except Exception as exc:
            failures.append({"code": "PDF_VALIDATION_FAILED", "message": str(exc)})
    checks = task.get("checks", {})
    pages = inspection.get("page_count")
    if deliverable.is_file() and isinstance(pages, int):
        if pages < int(checks.get("min_pages", 1)):
            failures.append({"code": "PAGE_COUNT_LOW", "actual": pages})
        maximum = checks.get("max_pages")
        if isinstance(maximum, int) and pages > maximum:
            failures.append({"code": "PAGE_COUNT_HIGH", "actual": pages})
    for marker in checks.get("text_markers", []):
        if marker not in text:
            failures.append({"code": "TEXT_MARKER_MISSING", "marker": marker})
    cursor = -1
    for marker in checks.get("ordered_text_markers", []):
        position = text.find(marker, cursor + 1)
        if position < 0:
            failures.append({"code": "TEXT_ORDER_INVALID", "marker": marker})
            break
        cursor = position
    claims = set((inspection.get("profile") or {}).get("claims", []))
    requested_claims = set(checks.get("any_profile_claims", []))
    if requested_claims and not claims.intersection(requested_claims):
        failures.append(
            {"code": "PROFILE_CLAIM_MISSING", "expected_any": sorted(requested_claims)}
        )
    prefixes = checks.get("any_profile_prefixes", [])
    if prefixes and not any(
        any(claim.startswith(prefix) for prefix in prefixes) for claim in claims
    ):
        failures.append({"code": "PROFILE_CLAIM_MISSING", "expected_prefixes": prefixes})
    identical_path = checks.get("byte_identical_to")
    deterministic = None
    if identical_path:
        rerun = task_root / identical_path
        deterministic = bool(
            deliverable.is_file()
            and rerun.is_file()
            and deliverable.read_bytes() == rerun.read_bytes()
        )
        if not deterministic:
            failures.append({"code": "DETERMINISM_FAILED", "comparison": str(rerun)})
    submission_path = task_root / "submission.json"
    submission = None
    if submission_path.is_file():
        try:
            submission = json.loads(submission_path.read_text(encoding="utf-8"))
        except Exception as exc:
            failures.append({"code": "SUBMISSION_METADATA_INVALID", "message": str(exc)})
        else:
            if (
                not isinstance(submission, Mapping)
                or submission.get("schema") != "agentdocbench.submission.v1"
            ):
                failures.append(
                    {
                        "code": "SUBMISSION_METADATA_INVALID",
                        "message": "submission.json must use agentdocbench.submission.v1",
                    }
                )
            metrics = (
                submission.get("metrics", {})
                if isinstance(submission, Mapping)
                else {}
            )
            if not isinstance(metrics, Mapping):
                failures.append(
                    {
                        "code": "SUBMISSION_METADATA_INVALID",
                        "message": "submission metrics must be an object",
                    }
                )
            else:
                for key in (
                    "agent_tokens",
                    "tool_calls",
                    "correction_loops",
                    "setup_failures",
                    "execution_ms",
                ):
                    value = metrics.get(key)
                    if value is not None and (
                        not isinstance(value, (int, float))
                        or isinstance(value, bool)
                        or value < 0
                    ):
                        failures.append(
                            {
                                "code": "SUBMISSION_METADATA_INVALID",
                                "message": f"submission metrics.{key} must be non-negative",
                            }
                        )
            first_pass = (
                submission.get("first_pass_success")
                if isinstance(submission, Mapping)
                else None
            )
            if first_pass is not None and not isinstance(first_pass, bool):
                failures.append(
                    {
                        "code": "SUBMISSION_METADATA_INVALID",
                        "message": "submission first_pass_success must be boolean",
                    }
                )
    elif checks.get("submission_required"):
        failures.append(
            {
                "code": "SUBMISSION_METADATA_MISSING",
                "path": str(submission_path),
            }
        )
    return {
        "id": task["id"],
        "ok": not failures,
        "artifact": (
            {
                "path": str(deliverable),
                "bytes": deliverable.stat().st_size,
                "sha256": hashlib.sha256(deliverable.read_bytes()).hexdigest(),
                "page_count": pages,
            }
            if deliverable.is_file()
            else None
        ),
        "profile_claims": sorted(claims),
        "deterministic": deterministic,
        "submission": submission,
        "visual_review": {"status": "pending", "score": None},
        "failures": failures,
    }


def score(workspace: Path, task_ids: list[str] | None = None) -> dict[str, Any]:
    import fullbleed

    registry = _tasks()
    root = workspace.resolve(strict=True)
    selected = task_ids or sorted(
        child.name for child in root.iterdir() if child.is_dir() and child.name in registry
    )
    results = []
    for task_id in selected:
        if task_id not in registry:
            raise ValueError(f"unknown task: {task_id}")
        results.append(_score_task(root / task_id, registry[task_id], fullbleed))
    passed = sum(1 for result in results if result["ok"])
    reported_keys = (
        "agent_tokens",
        "tool_calls",
        "correction_loops",
        "setup_failures",
        "execution_ms",
    )
    reported_totals: dict[str, float | int] = {}
    reports = 0
    first_pass_reported = 0
    first_pass_successes = 0
    for result in results:
        submission = result.get("submission")
        if (
            not isinstance(submission, Mapping)
            or submission.get("schema") != "agentdocbench.submission.v1"
        ):
            continue
        reports += 1
        metrics = submission.get("metrics")
        if isinstance(metrics, Mapping):
            for key in reported_keys:
                value = metrics.get(key)
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    reported_totals[key] = reported_totals.get(key, 0) + value
        first_pass = submission.get("first_pass_success")
        if isinstance(first_pass, bool):
            first_pass_reported += 1
            first_pass_successes += int(first_pass)
    return {
        "schema": "agentdocbench.result.v1",
        "ok": passed == len(results) and bool(results),
        "workspace": str(root),
        "tasks": results,
        "metrics": {
            "total": len(results),
            "passed": passed,
            "failed": len(results) - passed,
            "reported_submissions": reports,
            "reported_totals": reported_totals,
            "first_pass_reported": first_pass_reported,
            "first_pass_successes": first_pass_successes,
        },
        "rubric_status": "initial-machine-checks; visual review pending",
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    actions = parser.add_subparsers(dest="action", required=True)
    for name in ("prepare", "score"):
        action = actions.add_parser(name)
        action.add_argument("--workspace", type=Path, required=True)
        action.add_argument("--task", action="append")
        action.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        payload = (
            prepare(arguments.workspace, arguments.task)
            if arguments.action == "prepare"
            else score(arguments.workspace, arguments.task)
        )
        if arguments.json:
            sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(
                f"[{'ok' if payload['ok'] else 'fail'}] {payload['schema']}\n"
            )
        return 0 if payload["ok"] else 1
    except Exception as exc:
        if arguments.json:
            sys.stdout.write(
                json.dumps(
                    {"schema": "agentdocbench.error.v1", "ok": False, "message": str(exc)},
                    ensure_ascii=True,
                )
                + "\n"
            )
        else:
            sys.stderr.write(f"[error] {exc}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
