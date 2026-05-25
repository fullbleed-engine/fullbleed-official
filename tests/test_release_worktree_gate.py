from __future__ import annotations

from tools import check_release_worktree


def test_release_worktree_summary_counts_tracked_and_untracked_entries() -> None:
    entries = check_release_worktree.parse_status_lines(
        [
            " M README.md",
            "M  src/lib.rs",
            "?? output/report.pdf",
            "R  old.txt -> docs/new.txt",
        ]
    )

    payload = check_release_worktree.summarize(entries, max_entries=2)

    assert payload["ok"] is False
    assert payload["counts"] == {
        "total": 4,
        "tracked_dirty": 3,
        "untracked": 1,
        "staged": 2,
        "unstaged": 1,
        "tracked_artifact_violations": 0,
    }
    assert payload["by_top_level"] == {
        "docs": 1,
        "output": 1,
        "README.md": 1,
        "src": 1,
    }
    assert len(payload["entries"]) == 2
    assert payload["truncated_entries"] == 2


def test_release_worktree_summary_accepts_clean_tree() -> None:
    payload = check_release_worktree.summarize([], max_entries=25)

    assert payload["ok"] is True
    assert payload["counts"]["total"] == 0
    assert payload["counts"]["tracked_artifact_violations"] == 0
    assert payload["by_top_level"] == {}


def test_release_worktree_summary_fails_on_tracked_artifact_violations() -> None:
    payload = check_release_worktree.summarize(
        [],
        max_entries=2,
        tracked_artifact_violations=[
            ".pytest_cache/v/cache/nodeids",
            "fullbleed_preflight_hot.log",
            "python/fullbleed/_fullbleed.cp311-win_amd64.pyd",
            "target/debug/libfullbleed.so",
        ],
    )

    assert payload["ok"] is False
    assert payload["counts"]["tracked_artifact_violations"] == 4
    assert payload["tracked_artifact_violations"] == [
        ".pytest_cache/v/cache/nodeids",
        "fullbleed_preflight_hot.log",
    ]
    assert payload["truncated_tracked_artifact_violations"] == 2


def test_release_worktree_identifies_forbidden_tracked_artifacts() -> None:
    assert check_release_worktree.is_forbidden_tracked_artifact(
        ".pytest_cache/v/cache/nodeids"
    )
    assert check_release_worktree.is_forbidden_tracked_artifact(
        "python/fullbleed/_fullbleed.cp311-win_amd64.pyd"
    )
    assert check_release_worktree.is_forbidden_tracked_artifact("libfullbleed.so")
    assert check_release_worktree.is_forbidden_tracked_artifact(
        "fullbleed_preflight_hot.log"
    )
    assert check_release_worktree.is_forbidden_tracked_artifact(
        "fullbleed_css_fixture_text_shadow_paint_1.jsonl"
    )
    assert not check_release_worktree.is_forbidden_tracked_artifact(
        "examples/canonical_reference/output/.gitignore"
    )
    assert not check_release_worktree.is_forbidden_tracked_artifact("src/lib.rs")
