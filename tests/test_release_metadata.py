"""Stable and prerelease registries must identify the same source version."""
from pathlib import Path
import re
import shutil

import pytest

from tools import check_release_metadata as release

ROOT = Path(__file__).resolve().parents[1]


def candidate(tmp_path, python_version, cargo_version):
    for name in (
        "Cargo.toml", "pyproject.toml", "Cargo.lock", "crates/fullbleed_audit_contract/Cargo.toml",
        ".github/workflows/release.yml", ".github/workflows/ci.yml",
        ".github/workflows/publish-crates.yml", ".github/workflows/pdfvt-release-gate.yml",
    ):
        target = tmp_path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, target)
    for filename, version in (("Cargo.toml", cargo_version), ("pyproject.toml", python_version)):
        path = tmp_path / filename
        path.write_text(re.sub(r'(?m)^version = "[^"]+"', f'version = "{version}"', path.read_text(), count=1), encoding="utf-8")
    lock = tmp_path / "Cargo.lock"
    lock.write_text(re.sub(r'(name = "fullbleed"\nversion = ")[^"]+', rf'\g<1>{cargo_version}', lock.read_text()), encoding="utf-8")
    workflow = tmp_path / ".github/workflows/ci.yml"
    workflow.write_text(re.sub(r'FULLBLEED_CI_VERSION: "[^"]+"', f'FULLBLEED_CI_VERSION: "{python_version}"', workflow.read_text()), encoding="utf-8")
    (tmp_path / "README.md").write_text(f"Wheel: fullbleed-{python_version}\n", encoding="utf-8")
    (tmp_path / "ReleaseNotes.MD").write_text(f"Release Notes - {python_version}\n", encoding="utf-8")
    runbook = tmp_path / f"docs/release/{python_version}-runbook.md"
    runbook.parent.mkdir(parents=True, exist_ok=True)
    runbook.write_text(f"Tag: v{python_version}\n", encoding="utf-8")
    return tmp_path


@pytest.mark.parametrize(("python_version", "cargo_version", "prerelease"), [
    ("2.5.26", "2.5.26", False),
    ("2.6.0a1", "2.6.0-alpha.1", True),
    ("2.6.0b2", "2.6.0-beta.2", True),
    ("2.6.0rc12", "2.6.0-rc.12", True),
    ("0.1.0a0", "0.1.0-alpha.0", True),
])
def test_release_versions_agree_across_registries(tmp_path, python_version, cargo_version, prerelease):
    report = release.run(candidate(tmp_path, python_version, cargo_version))
    assert report["ok"], report["flags"]
    assert report["expected_version"] == python_version
    assert report["expected_cargo_version"] == cargo_version
    assert report["expected_tag"] == "v" + python_version
    assert report["prerelease"] is prerelease


@pytest.mark.parametrize(("filename", "before", "after", "code"), [
    ("Cargo.toml", "2.6.0-alpha.1", "2.6.0-alpha.2", "REL_VERSION_MISMATCH"),
    ("Cargo.toml", "2.6.0-alpha.1", "2.6.0", "REL_VERSION_MISMATCH"),
    ("pyproject.toml", "2.6.0a1", "2.6.0b1", "REL_VERSION_MISMATCH"),
    ("Cargo.lock", "2.6.0-alpha.1", "2.6.0", "REL_LOCK_MISMATCH"),
    (".github/workflows/ci.yml", "2.6.0a1", "2.6.0", "REL_CI_VERSION_MISMATCH"),
    ("docs/release/2.6.0a1-runbook.md", "v2.6.0a1", "v2.6.0", "REL_DOC_VERSION_MISMATCH"),
])
def test_prerelease_mismatches_are_release_gate_failures(tmp_path, filename, before, after, code):
    folder = candidate(tmp_path, "2.6.0a1", "2.6.0-alpha.1")
    path = folder / filename
    path.write_text(path.read_text().replace(before, after), encoding="utf-8")
    report = release.run(folder, "2.6.0a1")
    assert not report["ok"]
    assert [(flag["code"], flag["target"]) for flag in report["flags"]] == [(code, filename)]


@pytest.mark.parametrize("version", [
    "2.6", "v2.6.0", "2.6.0-alpha.1", "2.6.0a", "2.6.0a01", "02.6.0a1",
    "2.6.0.post1", "2.6.0.dev1", "2.6.0+private", "../../outside", "2.6.0a1\n",
])
def test_only_canonical_release_identifiers_are_allowed(tmp_path, version):
    folder = candidate(tmp_path, "2.6.0a1", "2.6.0-alpha.1")
    report = release.run(folder, version)
    assert not report["ok"] and report["expected_tag"] is None
    assert any(flag["code"] == "REL_VERSION_INVALID" for flag in report["flags"])


def test_requested_tag_version_cannot_disagree_with_the_checked_out_source(tmp_path):
    folder = candidate(tmp_path, "2.6.0a1", "2.6.0-alpha.1")
    report = release.run(folder, "2.6.0a2")
    assert not report["ok"]
    assert {flag["target"] for flag in report["flags"] if flag["code"] == "REL_VERSION_MISMATCH"} == {"Cargo.toml", "pyproject.toml"}
