#!/usr/bin/env python
"""Check that Cargo, PyPI, docs, lockfiles, and release automation agree."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


SUPPORTED_PYTHONS = ("3.10", "3.11", "3.12", "3.13", "3.14")
REQUIRED_WHEEL_TARGETS = (
    "manylinux-x86_64",
    "manylinux-x86",
    "manylinux-aarch64",
    "manylinux-armv7",
    "manylinux-s390x",
    "manylinux-ppc64le",
    "musllinux-x86_64",
    "musllinux-x86",
    "musllinux-aarch64",
    "musllinux-armv7",
    "windows-x86_64",
    "windows-x86",
    "windows-arm64",
    "macos-x86_64",
    "macos-aarch64",
)
REQUIRED_CARGO_PINS = {"subsetter": "0.2.6"}
BANNED_PYTHON_DISTRIBUTIONS = ("fastapi", "uvicorn", "httpx", "maturin")
BANNED_DIRECT_RUST_DEPENDENCIES = (
    "base64",
    "fixed",
    "image",
    "kuchiki",
    "libm",
    "lightningcss",
    "lopdf",
    "parcel_selectors",
    "pyo3",
    "rayon",
    "resvg",
    "roxmltree",
    "rustybuzz",
    "serde_json",
    "sha2",
    "tiny-skia",
    "time",
    "ttf-parser",
)


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _table_string(text: str, table: str, key: str) -> str | None:
    current_table = None
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if line.startswith("[") and line.endswith("]"):
            current_table = line[1:-1].strip()
            continue
        if current_table != table:
            continue
        match = re.match(rf"{re.escape(key)}\s*=\s*[\"']([^\"']+)[\"']", line)
        if match:
            return match.group(1)
    return None


def _dependency_version(text: str, dependency: str) -> str | None:
    match = re.search(
        rf"(?m)^\s*{re.escape(dependency)}\s*=\s*\{{[^}}]*\bversion\s*=\s*[\"']([^\"']+)",
        text,
    )
    return match.group(1) if match else None


def _dependency_requirement(text: str, dependency: str) -> str | None:
    string_match = re.search(
        rf"(?m)^\s*{re.escape(dependency)}\s*=\s*[\"']([^\"']+)[\"']",
        text,
    )
    if string_match:
        return string_match.group(1)
    return _dependency_version(text, dependency)


def _lock_version(text: str, package: str) -> str | None:
    pattern = re.compile(
        rf'(?ms)^\[\[package\]\]\s+name = "{re.escape(package)}"\s+version = "([^"]+)"'
    )
    match = pattern.search(text)
    return match.group(1) if match else None


def _lock_package_names(text: str) -> set[str]:
    return set(re.findall(r'(?m)^name = "([^"]+)"$', text))


def _dependency_names(text: str) -> set[str]:
    names: set[str] = set()
    current_table = ""
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if line.startswith("[") and line.endswith("]"):
            current_table = line[1:-1].strip()
            continue
        if current_table == "dependencies":
            match = re.match(r'([A-Za-z0-9_-]+)\s*=', line)
            if match:
                names.add(match.group(1))
    return names


def _flag(flags: list[dict[str, str]], code: str, target: str, message: str) -> None:
    flags.append({"code": code, "target": target, "message": message})


def run(repo_root: Path, expected_version: str | None = None) -> dict[str, Any]:
    cargo_path = repo_root / "Cargo.toml"
    pyproject_path = repo_root / "pyproject.toml"
    audit_path = repo_root / "crates" / "fullbleed_audit_contract" / "Cargo.toml"
    lock_path = repo_root / "Cargo.lock"
    workflow_path = repo_root / ".github" / "workflows" / "release.yml"
    ci_workflow_path = repo_root / ".github" / "workflows" / "ci.yml"
    crates_workflow_path = (
        repo_root / ".github" / "workflows" / "publish-crates.yml"
    )

    cargo = _read(cargo_path)
    pyproject = _read(pyproject_path)
    audit = _read(audit_path)
    lock = _read(lock_path)
    workflow = _read(workflow_path) if workflow_path.exists() else ""
    ci_workflow = _read(ci_workflow_path) if ci_workflow_path.exists() else ""
    crates_workflow = (
        _read(crates_workflow_path) if crates_workflow_path.exists() else ""
    )

    cargo_version = _table_string(cargo, "package", "version")
    python_version = _table_string(pyproject, "project", "version")
    audit_version = _table_string(audit, "package", "version")
    expected_version = expected_version or cargo_version
    expected_tag = f"v{expected_version}" if expected_version else None
    audit_dependency = _dependency_requirement(cargo, "fullbleed_audit_contract")

    flags: list[dict[str, str]] = []
    if not expected_version:
        _flag(flags, "REL_VERSION_MISSING", "Cargo.toml", "Main package version is missing")
    elif f'FULLBLEED_CI_VERSION: "{expected_version}"' not in ci_workflow:
        _flag(
            flags,
            "REL_CI_VERSION_MISMATCH",
            ".github/workflows/ci.yml",
            f"CI wheel smoke version does not match {expected_version}",
        )
    for target, observed in (
        ("Cargo.toml", cargo_version),
        ("pyproject.toml", python_version),
    ):
        if observed != expected_version:
            _flag(
                flags,
                "REL_VERSION_MISMATCH",
                target,
                f"Expected {expected_version}, observed {observed}",
            )

    for target, text, table in (
        ("Cargo.toml", cargo, "package"),
        ("pyproject.toml", pyproject, "project"),
        ("crates/fullbleed_audit_contract/Cargo.toml", audit, "package"),
    ):
        observed = _table_string(text, table, "license")
        if observed != "MIT":
            _flag(
                flags,
                "REL_LICENSE_MISMATCH",
                target,
                f"Expected MIT, observed {observed}",
            )

    if audit_dependency != audit_version:
        _flag(
            flags,
            "REL_DEPENDENCY_MISMATCH",
            "Cargo.toml",
            f"Audit dependency requires {audit_dependency}, package is {audit_version}",
        )

    for dependency, expected_requirement in REQUIRED_CARGO_PINS.items():
        observed = _dependency_requirement(cargo, dependency)
        if observed != expected_requirement:
            _flag(
                flags,
                "REL_DEPENDENCY_PIN_MISMATCH",
                "Cargo.toml",
                f"{dependency} expected {expected_requirement}, observed {observed}",
            )

    for dependency in BANNED_DIRECT_RUST_DEPENDENCIES:
        for target, manifest in (
            ("Cargo.toml", cargo),
            ("crates/fullbleed_audit_contract/Cargo.toml", audit),
        ):
            if _dependency_requirement(manifest, dependency) is not None:
                _flag(
                    flags,
                    "REL_RUST_DEPENDENCY_PRESENT",
                    target,
                    f"Removed direct Rust dependency is declared: {dependency}",
                )

    expected_manifest_dependencies = {
        "Cargo.toml": {"fullbleed_audit_contract", "subsetter"},
        "crates/fullbleed_audit_contract/Cargo.toml": set(),
    }
    for target, manifest in (
        ("Cargo.toml", cargo),
        ("crates/fullbleed_audit_contract/Cargo.toml", audit),
    ):
        observed = _dependency_names(manifest)
        expected = expected_manifest_dependencies[target]
        if observed != expected:
            _flag(
                flags,
                "REL_RUST_GRAPH_MISMATCH",
                target,
                f"Expected dependencies {sorted(expected)}, observed {sorted(observed)}",
            )

    lock_packages = _lock_package_names(lock)
    expected_lock_packages = {
        "fullbleed",
        "fullbleed_audit_contract",
        "rustc-hash",
        "subsetter",
    }
    if lock_packages != expected_lock_packages:
        _flag(
            flags,
            "REL_LOCK_GRAPH_MISMATCH",
            "Cargo.lock",
            f"Expected packages {sorted(expected_lock_packages)}, observed {sorted(lock_packages)}",
        )

    for package, expected in (
        ("fullbleed", expected_version),
        ("fullbleed_audit_contract", audit_version),
    ):
        observed = _lock_version(lock, package)
        if observed != expected:
            _flag(
                flags,
                "REL_LOCK_MISMATCH",
                "Cargo.lock",
                f"{package} expected {expected}, observed {observed}",
            )

    requires_python = _table_string(pyproject, "project", "requires-python")
    if requires_python != ">=3.10":
        _flag(
            flags,
            "REL_PYTHON_RANGE_MISMATCH",
            "pyproject.toml",
            f"Expected >=3.10, observed {requires_python}",
        )
    build_backend = _table_string(pyproject, "build-system", "build-backend")
    if build_backend != "fullbleed_build_backend":
        _flag(
            flags,
            "REL_BUILD_BACKEND_MISMATCH",
            "pyproject.toml",
            f"Expected fullbleed_build_backend, observed {build_backend}",
        )
    if not re.search(r"(?m)^requires\s*=\s*\[\s*\]\s*$", pyproject):
        _flag(
            flags,
            "REL_BUILD_REQUIREMENTS_PRESENT",
            "pyproject.toml",
            "Python build-system requirements must remain empty",
        )
    if not re.search(
        r'(?m)^backend-path\s*=\s*\[\s*["\']build_backend["\']\s*\]\s*$',
        pyproject,
    ):
        _flag(
            flags,
            "REL_BUILD_BACKEND_PATH_MISSING",
            "pyproject.toml",
            "The in-tree build backend path is missing",
        )
    abi_tag = _table_string(pyproject, "tool.fullbleed-build", "abi-tag")
    if abi_tag != "cp310-abi3":
        _flag(
            flags,
            "REL_PYTHON_ABI_TAG_MISMATCH",
            "pyproject.toml",
            f"Expected cp310-abi3, observed {abi_tag}",
        )
    for distribution in BANNED_PYTHON_DISTRIBUTIONS:
        if re.search(rf'(?i)["\']{re.escape(distribution)}(?:[<>=!~;\s"\']|$)', pyproject):
            _flag(
                flags,
                "REL_PYTHON_DEPENDENCY_PRESENT",
                "pyproject.toml",
                f"Removed Python dependency is declared: {distribution}",
            )
    for python_version_supported in SUPPORTED_PYTHONS:
        marker = f'"Programming Language :: Python :: {python_version_supported}"'
        if marker not in pyproject:
            _flag(
                flags,
                "REL_PYTHON_CLASSIFIER_MISSING",
                "pyproject.toml",
                f"Missing Python {python_version_supported} classifier",
            )
        if f'"{python_version_supported}"' not in workflow:
            _flag(
                flags,
                "REL_PYTHON_CI_MISSING",
                ".github/workflows/release.yml",
                f"Missing Python {python_version_supported} smoke coverage",
            )

    for wheel_target in REQUIRED_WHEEL_TARGETS:
        if not re.search(
            rf"(?m)^\s+(?:-\s+)?label:\s+{re.escape(wheel_target)}\s*$",
            workflow,
        ):
            _flag(
                flags,
                "REL_WHEEL_BUILD_TARGET_MISSING",
                ".github/workflows/release.yml",
                f"Missing wheel build target: {wheel_target}",
            )
        if not re.search(
            rf"(?m)^\s+(?:-\s+)?artifact:\s+{re.escape(wheel_target)}\s*$",
            workflow,
        ):
            _flag(
                flags,
                "REL_WHEEL_SMOKE_TARGET_MISSING",
                ".github/workflows/release.yml",
                f"Missing installed-wheel smoke target: {wheel_target}",
            )

    for marker in (
        'environment: "pypi"',
        "publish_pypi:",
        "build_backend/fullbleed_build_backend.py",
        "tools/build_linux_wheel_container.sh",
        "compatibility: manylinux2014",
        "compatibility: musllinux_1_2",
        "windows-11-arm",
        "target: s390x-unknown-linux-gnu",
        "target: powerpc64le-unknown-linux-gnu",
        "ghcr.io/rust-cross/manylinux2014-cross:armv7",
        "docker/setup-qemu-action@c7c53464625b32c7a7e944ae62b3e17d2b600130",
        "pypa/gh-action-pypi-publish@cef221092ed1bacb1cc03d23a2d87d1d172e277b",
        "tools/check_release_worktree.py",
        "tools/smoke_installed_package.py",
        "python -m pytest -q",
        "tools/smoke_crates_consumer.py",
        "rustup toolchain install 1.85.0",
    ):
        if marker not in workflow:
            _flag(
                flags,
                "REL_AUTOMATION_MARKER_MISSING",
                ".github/workflows/release.yml",
                f"Missing release marker: {marker}",
            )

    for marker in (
        "name: Publish crates.io",
        "name: crates-io",
        "id-token: write",
        "tools/check_license_integrity.py",
        "cargo publish --locked --dry-run",
        "rust-lang/crates-io-auth-action@c6f97d42243bad5fab37ca0427f495c86d5b1a18",
        "tools/smoke_crates_consumer.py",
        "rustup toolchain install 1.85.0",
    ):
        if marker not in crates_workflow:
            _flag(
                flags,
                "REL_AUTOMATION_MARKER_MISSING",
                ".github/workflows/publish-crates.yml",
                f"Missing crates.io release marker: {marker}",
            )

    for relative_path, marker in (
        ("README.md", f"fullbleed-{expected_version}"),
        ("ReleaseNotes.MD", f"Release Notes - {expected_version}"),
        (f"docs/release/{expected_version}-runbook.md", expected_tag or ""),
    ):
        path = repo_root / relative_path
        if not path.exists() or marker not in _read(path):
            _flag(
                flags,
                "REL_DOC_VERSION_MISMATCH",
                relative_path,
                f"Missing expected release marker: {marker}",
            )

    return {
        "schema": "fullbleed.release_metadata.v1",
        "ok": not flags,
        "expected_version": expected_version,
        "expected_tag": expected_tag,
        "supported_python": list(SUPPORTED_PYTHONS),
        "wheel_targets": list(REQUIRED_WHEEL_TARGETS),
        "packages": {
            "fullbleed": cargo_version,
            "python": python_version,
            "fullbleed_audit_contract": audit_version,
        },
        "flags": flags,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-version")
    parser.add_argument(
        "--repo-root",
        default=str(Path(__file__).resolve().parents[1]),
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    report = run(Path(args.repo_root), args.expected_version)
    if args.json:
        print(json.dumps(report, ensure_ascii=True))
    else:
        print(f"ok: {report['ok']}")
        print(f"expected_version: {report['expected_version']}")
        print(f"expected_tag: {report['expected_tag']}")
        if report["flags"]:
            print("flags:")
            for flag in report["flags"]:
                print(f"  - {flag['code']} [{flag['target']}]: {flag['message']}")
        else:
            print("flags: []")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
