from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path


SCHEMA = "fullbleed.release_worktree.v1"
FORBIDDEN_TRACKED_ARTIFACTS = {
    ".dll",
    ".dylib",
    ".pyd",
    ".so",
}
FORBIDDEN_TRACKED_PATHS = {
    "fullbleed_preflight.jit",
    "fullbleed_preflight.perf",
    "fullbleed_preflight_hot.log",
}


def _top_level(path: str) -> str:
    normalized = path.replace("\\", "/")
    if " -> " in normalized:
        normalized = normalized.split(" -> ", 1)[1]
    first = normalized.split("/", 1)[0].strip()
    return first or "<root>"


def parse_status_lines(lines: list[str]) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    for raw in lines:
        line = raw.rstrip("\n")
        if not line:
            continue
        if len(line) < 4:
            status = line.strip()
            path = ""
        else:
            status = line[:2]
            path = line[3:]
        tracked = status != "??"
        entries.append(
            {
                "status": status,
                "path": path,
                "top_level": _top_level(path),
                "tracked": tracked,
                "untracked": not tracked,
                "staged": tracked and status[0] != " ",
                "unstaged": tracked and status[1] != " ",
            }
        )
    return entries


def is_forbidden_tracked_artifact(path: str) -> bool:
    normalized = path.replace("\\", "/")
    suffix = Path(normalized).suffix.lower()
    name = Path(normalized).name
    return (
        normalized.startswith(".pytest_cache/")
        or suffix in FORBIDDEN_TRACKED_ARTIFACTS
        or normalized in FORBIDDEN_TRACKED_PATHS
        or (name.startswith("fullbleed_css_fixture_") and suffix == ".jsonl")
    )


def summarize(
    entries: list[dict[str, object]],
    max_entries: int,
    tracked_artifact_violations: list[str] | None = None,
) -> dict[str, object]:
    artifact_violations = tracked_artifact_violations or []
    by_top_level = Counter(str(entry["top_level"]) for entry in entries)
    by_status = Counter(str(entry["status"]) for entry in entries)
    tracked_dirty = sum(1 for entry in entries if entry["tracked"])
    untracked = sum(1 for entry in entries if entry["untracked"])
    staged = sum(1 for entry in entries if entry["staged"])
    unstaged = sum(1 for entry in entries if entry["unstaged"])
    return {
        "schema": SCHEMA,
        "ok": not entries and not artifact_violations,
        "counts": {
            "total": len(entries),
            "tracked_dirty": tracked_dirty,
            "untracked": untracked,
            "staged": staged,
            "unstaged": unstaged,
            "tracked_artifact_violations": len(artifact_violations),
        },
        "by_status": dict(sorted(by_status.items())),
        "by_top_level": dict(
            sorted(by_top_level.items(), key=lambda item: (-item[1], item[0]))
        ),
        "entries": entries[:max_entries],
        "truncated_entries": max(0, len(entries) - max_entries),
        "tracked_artifact_violations": artifact_violations[:max_entries],
        "truncated_tracked_artifact_violations": max(
            0, len(artifact_violations) - max_entries
        ),
        "release_requirement": (
            "Release worktree must be clean and must not track generated "
            "artifact files before final tag/publish."
        ),
    }


def git_status(repo: Path) -> list[str]:
    proc = subprocess.run(
        ["git", "status", "--short"],
        cwd=repo,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or "git status failed")
    return proc.stdout.splitlines()


def git_tracked_files(repo: Path) -> list[str]:
    proc = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repo,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            proc.stderr.decode("utf-8", errors="replace").strip()
            or "git ls-files failed"
        )
    return [
        item.decode("utf-8", errors="replace")
        for item in proc.stdout.split(b"\0")
        if item
    ]


def tracked_artifact_violations(repo: Path) -> list[str]:
    return sorted(path for path in git_tracked_files(repo) if is_forbidden_tracked_artifact(path))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail unless the release worktree is clean."
    )
    parser.add_argument("--repo", default=".", help="Repository root to inspect.")
    parser.add_argument("--json", action="store_true", help="Emit JSON output.")
    parser.add_argument(
        "--max-entries",
        type=int,
        default=25,
        help="Maximum dirty entries to include in the report.",
    )
    args = parser.parse_args(argv)

    repo = Path(args.repo).resolve()
    try:
        payload = summarize(
            parse_status_lines(git_status(repo)),
            args.max_entries,
            tracked_artifact_violations(repo),
        )
    except Exception as exc:
        payload = {
            "schema": SCHEMA,
            "ok": False,
            "error": str(exc),
            "release_requirement": (
                "Release worktree must be clean and must not track generated "
                "artifact files before final tag/publish."
            ),
        }
        if args.json:
            print(json.dumps(payload, ensure_ascii=True))
        else:
            print(f"release worktree check failed: {exc}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(payload, ensure_ascii=True))
    elif payload["ok"]:
        print("release worktree clean")
    else:
        counts = payload["counts"]
        assert isinstance(counts, dict)
        print(
            "release worktree dirty: "
            f"{counts['total']} total, "
            f"{counts['tracked_dirty']} tracked, "
            f"{counts['untracked']} untracked"
        )
        print("top-level dirty counts:")
        by_top_level = payload["by_top_level"]
        assert isinstance(by_top_level, dict)
        for name, count in by_top_level.items():
            print(f"  {name}: {count}")

    return 0 if payload["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
