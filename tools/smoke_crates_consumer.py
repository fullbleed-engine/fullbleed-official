#!/usr/bin/env python
"""Compile the packaged Fullbleed crate from a lockfile-free consumer project."""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
import tomllib
from pathlib import Path
from typing import Any


FORBIDDEN_CONSUMER_DEPENDENCIES = (
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


class SmokeFailure(RuntimeError):
    """A concise, user-facing smoke-test failure."""


def _read_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def _tail(value: str, *, lines: int = 80) -> str:
    return "\n".join(value.splitlines()[-lines:])


def _run(command: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> None:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode == 0:
        return
    rendered = " ".join(command)
    raise SmokeFailure(
        f"{rendered} failed with exit code {result.returncode}\n"
        f"stdout:\n{_tail(result.stdout)}\n"
        f"stderr:\n{_tail(result.stderr)}"
    )


def _dependency_requirement(manifest: dict[str, Any], name: str) -> str:
    dependency = manifest.get("dependencies", {}).get(name)
    if isinstance(dependency, str):
        return dependency
    if isinstance(dependency, dict) and isinstance(dependency.get("version"), str):
        return dependency["version"]
    raise SmokeFailure(f"Cargo.toml is missing a version requirement for {name}")


def _exact_dependency_version(manifest: dict[str, Any], name: str) -> str:
    requirement = _dependency_requirement(manifest, name)
    if not requirement.startswith("="):
        raise SmokeFailure(
            f"{name} must use an exact Cargo requirement; observed {requirement!r}"
        )
    return requirement[1:]


def _safe_extract(archive: Path, destination: Path) -> None:
    root = destination.resolve()
    with tarfile.open(archive, mode="r:gz") as package:
        members = package.getmembers()
        for member in members:
            relative = Path(member.name)
            if (
                relative.is_absolute()
                or ".." in relative.parts
                or member.issym()
                or member.islnk()
            ):
                raise SmokeFailure(
                    f"Refusing unsafe Cargo package entry: {member.name!r}"
                )
            target = (destination / relative).resolve()
            try:
                target.relative_to(root)
            except ValueError as error:
                raise SmokeFailure(
                    f"Cargo package entry escapes extraction root: {member.name!r}"
                ) from error
        package.extractall(destination, members=members)


def _locked_package(lock: dict[str, Any], name: str) -> dict[str, Any]:
    matches = [package for package in lock.get("package", []) if package["name"] == name]
    if len(matches) != 1:
        versions = [package.get("version") for package in matches]
        raise SmokeFailure(
            f"Expected exactly one {name} package in consumer lockfile; found {versions}"
        )
    return matches[0]


def run(repo_root: Path, expected_version: str | None = None) -> dict[str, Any]:
    repo_root = repo_root.resolve()
    manifest_path = repo_root / "Cargo.toml"
    manifest = _read_toml(manifest_path)
    package_version = manifest["package"]["version"]
    expected_version = expected_version or package_version
    if package_version != expected_version:
        raise SmokeFailure(
            f"Cargo package version is {package_version}, expected {expected_version}"
        )

    audit_version = _dependency_requirement(
        manifest, "fullbleed_audit_contract"
    ).lstrip("=")
    subsetter_version = _dependency_requirement(manifest, "subsetter").lstrip("=")
    repository_lock = _read_toml(repo_root / "Cargo.lock")
    rustc_hash_version = _locked_package(repository_lock, "rustc-hash")["version"]
    audit_root = repo_root / "crates" / "fullbleed_audit_contract"
    audit_manifest = _read_toml(audit_root / "Cargo.toml")
    if audit_manifest["package"]["version"] != audit_version:
        raise SmokeFailure(
            "Root dependency and audit-contract package versions disagree: "
            f"{audit_version} != {audit_manifest['package']['version']}"
        )

    _run(
        [
            "cargo",
            "package",
            "--locked",
            "--no-verify",
            "--allow-dirty",
        ],
        cwd=audit_root,
    )
    audit_archive = (
        audit_root
        / "target"
        / "package"
        / f"fullbleed_audit_contract-{audit_version}.crate"
    )
    if not audit_archive.is_file():
        raise SmokeFailure(f"Cargo package was not created: {audit_archive}")

    _run(
        [
            "cargo",
            "package",
            "--locked",
            "--no-verify",
            "--allow-dirty",
            "--config",
            "patch.crates-io.fullbleed_audit_contract.path="
            + json.dumps(audit_root.as_posix()),
        ],
        cwd=repo_root,
    )
    archive = repo_root / "target" / "package" / f"fullbleed-{package_version}.crate"
    if not archive.is_file():
        raise SmokeFailure(f"Cargo package was not created: {archive}")

    with tempfile.TemporaryDirectory(prefix="fullbleed-crates-consumer-") as raw_temp:
        temp_root = Path(raw_temp)
        extracted_root = temp_root / "package"
        extracted_root.mkdir()
        _safe_extract(archive, extracted_root)
        packaged_crate = extracted_root / f"fullbleed-{package_version}"
        if not (packaged_crate / "Cargo.toml").is_file():
            raise SmokeFailure(
                f"Extracted Cargo package is missing Cargo.toml: {packaged_crate}"
            )

        extracted_audit_root = temp_root / "audit-package"
        extracted_audit_root.mkdir()
        _safe_extract(audit_archive, extracted_audit_root)
        packaged_audit = (
            extracted_audit_root / f"fullbleed_audit_contract-{audit_version}"
        )
        if not (packaged_audit / "Cargo.toml").is_file():
            raise SmokeFailure(
                f"Extracted audit package is missing Cargo.toml: {packaged_audit}"
            )

        consumer = temp_root / "consumer"
        (consumer / "src").mkdir(parents=True)
        packaged_path = json.dumps(packaged_crate.as_posix())
        packaged_audit_path = json.dumps(packaged_audit.as_posix())
        (consumer / "Cargo.toml").write_text(
            "\n".join(
                (
                    "[package]",
                    'name = "fullbleed_external_consumer_smoke"',
                    'version = "0.0.0"',
                    'edition = "2024"',
                    "publish = false",
                    "",
                    "[dependencies]",
                    f"fullbleed = {{ path = {packaged_path} }}",
                    "",
                    "[patch.crates-io]",
                    "fullbleed_audit_contract = "
                    f"{{ path = {packaged_audit_path} }}",
                    "",
                )
            ),
            encoding="utf-8",
        )
        (consumer / "src" / "main.rs").write_text(
            "fn main() {}\n",
            encoding="utf-8",
        )

        cargo_env = os.environ.copy()
        cargo_env["CARGO_INCREMENTAL"] = "0"
        cargo_env["CARGO_TARGET_DIR"] = str(temp_root / "target")
        cargo_env["CARGO_TERM_COLOR"] = "never"
        _run(
            [
                "cargo",
                "check",
                "--manifest-path",
                str(consumer / "Cargo.toml"),
            ],
            cwd=consumer,
            env=cargo_env,
        )

        consumer_lock = _read_toml(consumer / "Cargo.lock")
        package_names = {
            package.get("name") for package in consumer_lock.get("package", [])
        }
        unexpected = sorted(package_names.intersection(FORBIDDEN_CONSUMER_DEPENDENCIES))
        if unexpected:
            raise SmokeFailure(
                "Removed dependencies reappeared in the fresh consumer lockfile: "
                + ", ".join(unexpected)
            )
        expected_package_names = {
            "fullbleed",
            "fullbleed_audit_contract",
            "fullbleed_external_consumer_smoke",
            "rustc-hash",
            "subsetter",
        }
        if package_names != expected_package_names:
            raise SmokeFailure(
                "Fresh consumer graph differs from the audited release graph: expected "
                f"{sorted(expected_package_names)}, observed {sorted(package_names)}"
            )
        resolved = {
            "fullbleed": _locked_package(consumer_lock, "fullbleed")["version"],
            "fullbleed_audit_contract": _locked_package(
                consumer_lock, "fullbleed_audit_contract"
            )["version"],
            "rustc-hash": _locked_package(consumer_lock, "rustc-hash")["version"],
            "subsetter": _locked_package(consumer_lock, "subsetter")["version"],
        }
        if resolved["fullbleed"] != package_version:
            raise SmokeFailure(
                f"Consumer resolved fullbleed {resolved['fullbleed']}, "
                f"expected {package_version}"
            )
        if resolved["fullbleed_audit_contract"] != audit_version:
            raise SmokeFailure(
                "Consumer resolved fullbleed_audit_contract "
                f"{resolved['fullbleed_audit_contract']}, expected {audit_version}"
            )
        if resolved["subsetter"] != subsetter_version:
            raise SmokeFailure(
                f"Consumer resolved subsetter {resolved['subsetter']}, "
                f"expected {subsetter_version}"
            )
        if resolved["rustc-hash"] != rustc_hash_version:
            raise SmokeFailure(
                f"Consumer resolved rustc-hash {resolved['rustc-hash']}, "
                f"expected {rustc_hash_version}"
            )

    return {
        "schema": "fullbleed.crates_consumer_smoke.v1",
        "ok": True,
        "package": f"fullbleed-{package_version}.crate",
        "audit_package": f"fullbleed_audit_contract-{audit_version}.crate",
        "consumer_lockfile": "fresh",
        "repository_lockfile_used_by_consumer": False,
        "removed_dependencies_absent": True,
        "resolved": resolved,
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

    try:
        report = run(Path(args.repo_root), args.expected_version)
    except (OSError, SmokeFailure, subprocess.SubprocessError, tarfile.TarError) as error:
        report = {
            "schema": "fullbleed.crates_consumer_smoke.v1",
            "ok": False,
            "error": str(error),
        }

    if args.json:
        print(json.dumps(report, ensure_ascii=True))
    elif report["ok"]:
        print(f"ok: {report['ok']}")
        print(f"package: {report['package']}")
        for name, version in report["resolved"].items():
            print(f"{name}: {version}")
    else:
        print(f"ok: {report['ok']}")
        print(f"error: {report['error']}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
