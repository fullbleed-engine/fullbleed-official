#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Retain the public wheel and matching release source used by compatibility CI."""
from __future__ import annotations

import argparse
from email.parser import BytesParser
import hashlib
import importlib.metadata
import importlib.util
import json
from pathlib import Path
import platform
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile


def verify_source_version(source: Path, expected_version: str) -> str:
    # Reuse the dependency-free parser used to build the wheel on Python 3.10.
    backend_path = Path(__file__).resolve().parents[1] / "build_backend/fullbleed_build_backend.py"
    spec = importlib.util.spec_from_file_location("_compatibility_build_backend", backend_path)
    backend = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(backend)
    project = backend._fallback_toml_loads(
        (source / "pyproject.toml").read_text(encoding="utf-8"))["project"]
    if project["name"] != "fullbleed" or project["version"] != expected_version:
        raise ValueError("Release source version does not match the installed package")
    return project["version"]


def verify_public_wheel(wheel: Path, expected_version: str, release: dict) -> dict:
    if release["info"]["name"] != "fullbleed" or release["info"]["version"] != expected_version:
        raise ValueError("Unexpected public release identity")
    matches = [row for row in release["urls"] if row["filename"] == wheel.name]
    if len(matches) != 1:
        raise ValueError("Wheel is not present in the public release")
    public = matches[0]
    digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
    if digest != public["digests"]["sha256"]:
        raise ValueError("Downloaded wheel differs from the public release")
    with zipfile.ZipFile(wheel) as archive:
        metadata_names = [name for name in archive.namelist() if name.endswith(".dist-info/METADATA")]
        if len(metadata_names) != 1:
            raise ValueError("Wheel must have one package metadata record")
        metadata = BytesParser().parsebytes(archive.read(metadata_names[0]))
        if metadata["Name"] != "fullbleed" or metadata["Version"] != expected_version:
            raise ValueError("Wheel metadata does not match the requested release")
        if metadata.get_all("Requires-Dist"):
            raise ValueError("Core wheel unexpectedly requires Python runtime dependencies")
        wheel_metadata = BytesParser().parsebytes(
            archive.read(metadata_names[0].removesuffix("METADATA") + "WHEEL"))
        tags = wheel_metadata.get_all("Tag", [])
        if not tags or not all(tag.startswith("cp310-abi3-") for tag in tags):
            raise ValueError("Expected the CPython 3.10 stable-ABI wheel")
    return {
        "filename": wheel.name, "bytes": wheel.stat().st_size, "sha256": digest,
        "url": public["url"], "requires_python": metadata["Requires-Python"],
        "requires_dist": [], "tags": tags,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--wheel-dir", type=Path, required=True)
    parser.add_argument("--expected-python", required=True, help="Baseline CPython major.minor")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()

    def git(*arguments):
        return subprocess.check_output(
            ["git", "-C", str(source), *arguments], text=True, encoding="utf-8").strip()

    if git("rev-parse", "HEAD") != args.commit or git("status", "--porcelain"):
        raise ValueError("Release source is not the clean pinned commit")
    source_version = verify_source_version(source, args.version)
    if platform.python_implementation() != "CPython" or \
            f"{sys.version_info.major}.{sys.version_info.minor}" != args.expected_python:
        raise ValueError("Unexpected baseline interpreter")
    distribution = importlib.metadata.distribution("fullbleed")
    if distribution.version != args.version or distribution.requires:
        raise ValueError("Unexpected installed Fullbleed package")
    wheels = list(args.wheel_dir.glob("*.whl"))
    if len(wheels) != 1:
        raise ValueError("Expected exactly one downloaded public wheel")
    url = "https://pypi.org/pypi/fullbleed/" + urllib.parse.quote(args.version, safe="") + "/json"
    with urllib.request.urlopen(url, timeout=60) as response:
        release = json.load(response)
    record = {
        "schema": "fullbleed.python_compatibility_source.v1", "ok": True,
        "version": args.version, "source_commit": args.commit, "source_clean": True,
        "source_version": source_version,
        "public_registry": url, "wheel": verify_public_wheel(wheels[0], args.version, release),
        "baseline_python": {"version": sys.version, "implementation": platform.python_implementation(),
                            "architecture": platform.machine(), "platform": sys.platform},
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(record))


if __name__ == "__main__":
    main()
