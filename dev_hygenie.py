#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class CleanupGroup:
    name: str
    description: str
    patterns: tuple[str, ...]
    default_enabled: bool = False


GROUPS: dict[str, CleanupGroup] = {
    "python-cache": CleanupGroup(
        name="python-cache",
        description="Python bytecode/cache artifacts",
        patterns=(
            "**/__pycache__",
            "**/*.pyc",
            "**/*.pyo",
            "**/*.pyc.*",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".hypothesis",
        ),
        default_enabled=True,
    ),
    "scratch": CleanupGroup(
        name="scratch",
        description="Local scratch/temp files produced during iterative runs",
        patterns=(
            "_tmp_*",
            "tmp_*",
            "fullbleed_preflight*.jit",
            "fullbleed_preflight*.perf",
            "fullbleed_preflight*.log",
            "local_manifest_remote_smoke.json",
        ),
        default_enabled=True,
    ),
    "build": CleanupGroup(
        name="build",
        description="Rust/Python build outputs (safe to regenerate)",
        patterns=(
            "target",
            "crates/**/target",
            ".maturin",
            "build",
            "dist/preflight_*",
        ),
        default_enabled=False,
    ),
    "examples-output": CleanupGroup(
        name="examples-output",
        description="Generated outputs inside examples/",
        patterns=(
            "examples/**/output/*",
            "output/_tmp_*",
        ),
        default_enabled=False,
    ),
    "venv": CleanupGroup(
        name="venv",
        description="Virtualenv folders",
        patterns=(
            ".venv",
            ".venv_*",
            "venv",
            "venv_*",
        ),
        default_enabled=False,
    ),
}

ALWAYS_EXCLUDE_PARTS = {".git"}
PRESERVE_BASENAMES = {".gitkeep", ".gitignore"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Repo cleanup utility. Dry-run by default.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--all", action="store_true", help="Include all cleanup groups")
    parser.add_argument(
        "--include",
        action="append",
        default=[],
        metavar="GROUPS",
        help="Comma-separated group list (repeatable), e.g. --include build,examples-output",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        metavar="GROUPS",
        help="Comma-separated group list to exclude",
    )
    parser.add_argument("--list-groups", action="store_true", help="Show available groups and exit")
    parser.add_argument("--apply", action="store_true", help="Apply deletions (otherwise dry-run)")
    parser.add_argument(
        "--yes",
        action="store_true",
        help="Required with --apply to confirm destructive actions",
    )
    parser.add_argument("--verbose", action="store_true")
    return parser.parse_args()


def split_group_args(values: list[str]) -> list[str]:
    out: list[str] = []
    for raw in values:
        for part in raw.split(","):
            group = part.strip()
            if group:
                out.append(group)
    return out


def rel_posix(path: Path, root: Path) -> str:
    return path.resolve().relative_to(root.resolve()).as_posix()


def get_git_tracked_files(root: Path) -> set[str]:
    try:
        proc = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            check=True,
            capture_output=True,
            text=False,
        )
    except Exception:
        return set()
    decoded = proc.stdout.decode("utf-8", errors="ignore")
    return {entry.replace("\\", "/") for entry in decoded.split("\x00") if entry}


def contains_tracked_descendant(rel_dir: str, tracked: set[str]) -> bool:
    prefix = rel_dir.rstrip("/") + "/"
    return any(path.startswith(prefix) for path in tracked)


def is_excluded_path(path: Path, root: Path, include_venv: bool) -> bool:
    try:
        rel = path.resolve().relative_to(root.resolve())
    except Exception:
        return True
    parts = set(rel.parts)
    if parts & ALWAYS_EXCLUDE_PARTS:
        return True
    if not include_venv:
        for part in rel.parts:
            if part == "venv" or part.startswith("venv_") or part == ".venv" or part.startswith(".venv_"):
                return True
    return False


def prune_descendants(paths: list[Path]) -> list[Path]:
    sorted_paths = sorted(
        paths,
        key=lambda p: (
            rel_depth(p),
            0 if p.is_dir() and not p.is_symlink() else 1,
            str(p),
        ),
    )
    kept: list[Path] = []
    kept_dirs: list[Path] = []
    for path in sorted_paths:
        if any(is_child_of(path, parent) for parent in kept_dirs):
            continue
        kept.append(path)
        if path.is_dir() and not path.is_symlink():
            kept_dirs.append(path)
    return kept


def is_child_of(path: Path, parent: Path) -> bool:
    try:
        path.resolve().relative_to(parent.resolve())
    except Exception:
        return False
    return path.resolve() != parent.resolve()


def rel_depth(path: Path) -> int:
    return len(path.resolve().parts)


def collect_group_paths(
    root: Path,
    group: CleanupGroup,
    tracked: set[str],
    include_venv: bool,
) -> tuple[list[Path], list[tuple[str, str]]]:
    candidates: list[Path] = []
    skipped: list[tuple[str, str]] = []
    seen: set[str] = set()

    for pattern in group.patterns:
        for path in root.glob(pattern):
            if not path.exists():
                continue
            if is_excluded_path(path, root, include_venv):
                continue
            rel = rel_posix(path, root)
            if rel in seen:
                continue
            seen.add(rel)

            if path.name in PRESERVE_BASENAMES:
                skipped.append((rel, "preserve basename"))
                continue

            if path.is_file() or path.is_symlink():
                if rel in tracked:
                    skipped.append((rel, "tracked file"))
                    continue
                candidates.append(path)
                continue

            if path.is_dir():
                if contains_tracked_descendant(rel, tracked):
                    skipped.append((rel, "tracked descendant exists"))
                    continue
                candidates.append(path)

    return prune_descendants(candidates), skipped


def bytes_for_path(path: Path) -> int:
    if path.is_file() or path.is_symlink():
        try:
            return path.stat().st_size
        except Exception:
            return 0
    if not path.is_dir():
        return 0
    total = 0
    for child in path.rglob("*"):
        if child.is_file():
            try:
                total += child.stat().st_size
            except Exception:
                continue
    return total


def delete_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
        return
    if path.is_dir():
        shutil.rmtree(path, ignore_errors=False)


def list_groups() -> None:
    print("Available cleanup groups:")
    for name in sorted(GROUPS):
        group = GROUPS[name]
        default = " (default)" if group.default_enabled else ""
        print(f"- {group.name}{default}: {group.description}")
        for pattern in group.patterns:
            print(f"    {pattern}")


def resolve_selected_groups(args: argparse.Namespace) -> list[str]:
    include_values = split_group_args(args.include)
    exclude_values = set(split_group_args(args.exclude))

    if args.all:
        selected = set(GROUPS.keys())
    elif include_values:
        selected = set(include_values)
    else:
        selected = {name for name, group in GROUPS.items() if group.default_enabled}

    unknown = sorted(selected.difference(GROUPS.keys()))
    if unknown:
        raise SystemExit(f"Unknown group(s): {', '.join(unknown)}")

    selected = selected.difference(exclude_values)
    return sorted(selected)


def main() -> int:
    args = parse_args()
    if args.list_groups:
        list_groups()
        return 0

    root = args.root.resolve()
    if not root.exists():
        raise SystemExit(f"Root does not exist: {root}")

    selected = resolve_selected_groups(args)
    include_venv = "venv" in selected
    tracked = get_git_tracked_files(root)

    group_to_paths: dict[str, list[Path]] = {}
    skipped: list[tuple[str, str]] = []
    all_paths: dict[str, Path] = {}

    for group_name in selected:
        group = GROUPS[group_name]
        paths, group_skipped = collect_group_paths(root, group, tracked, include_venv)
        group_to_paths[group_name] = paths
        skipped.extend(group_skipped)
        for path in paths:
            all_paths.setdefault(rel_posix(path, root), path)

    final_paths = sorted(
        all_paths.values(),
        key=lambda p: (rel_depth(p), 0 if p.is_dir() and not p.is_symlink() else 1, str(p)),
        reverse=True,
    )
    total_bytes = sum(bytes_for_path(path) for path in final_paths)
    file_count = sum(1 for path in final_paths if path.is_file() or path.is_symlink())
    dir_count = sum(1 for path in final_paths if path.is_dir() and not path.is_symlink())

    mode = "APPLY" if args.apply else "DRY-RUN"
    print(f"[dev_hygenie] root: {root}")
    print(f"[dev_hygenie] mode: {mode}")
    print(f"[dev_hygenie] groups: {', '.join(selected) if selected else '(none)'}")
    print(
        f"[dev_hygenie] candidates: {len(final_paths)} "
        f"(files={file_count}, dirs={dir_count}, bytes={total_bytes})"
    )

    for group_name in selected:
        print(f"  - {group_name}: {len(group_to_paths[group_name])} path(s)")

    if skipped:
        print(f"[dev_hygenie] skipped: {len(skipped)} path(s) due to protection rules")
        if args.verbose:
            for rel, reason in skipped:
                print(f"    skip {rel} ({reason})")

    if args.verbose or not args.apply:
        for path in final_paths:
            try:
                rel = rel_posix(path, root)
            except Exception:
                rel = str(path)
            print(f"    {rel}")

    if not args.apply:
        print("[dev_hygenie] dry-run complete. Re-run with --apply --yes to delete.")
        return 0

    if args.apply and not args.yes:
        raise SystemExit("Refusing to delete without --yes. Use: --apply --yes")

    errors: list[tuple[str, str]] = []
    deleted = 0
    for path in final_paths:
        try:
            delete_path(path)
            deleted += 1
        except Exception as exc:
            errors.append((str(path), f"{type(exc).__name__}: {exc}"))

    print(f"[dev_hygenie] deleted: {deleted}/{len(final_paths)} path(s)")
    if errors:
        print(f"[dev_hygenie] errors: {len(errors)}")
        for path, msg in errors:
            print(f"    {path}: {msg}")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
