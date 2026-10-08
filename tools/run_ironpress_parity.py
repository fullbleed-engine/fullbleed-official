#!/usr/bin/env python3
"""Run the pinned IronPress parity corpus through FullBleed.

Only IronPress's candidate-renderer boundary is patched. Fixtures, manifests,
browser PDF oracles, rasterization, comparison, and reporting remain upstream.
"""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
from email.parser import BytesParser
import fnmatch
import hashlib
import json
import re
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Sequence
import uuid
import zipfile


IRONPRESS_REPOSITORY = "https://github.com/gastongouron/ironpress.git"
IRONPRESS_COMMIT = "0d1e53b6d8174d0a5059a8696c24e62759381f6d"
IMAGE_TAG = "fullbleed-ironpress-parity:0d1e53b6"
LINUX_WHEEL_PATTERNS = (
    "fullbleed-*-cp310-abi3-manylinux*_x86_64.whl",
    "fullbleed-*-cp310-abi3-linux_x86_64.whl",
)
CORPUS_FIXTURES = 1662


def run(
    command: Sequence[str],
    *,
    cwd: Path | None = None,
    capture: bool = False,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    printable = subprocess.list2cmdline([str(part) for part in command])
    print(f"+ {printable}", file=sys.stderr, flush=True)
    try:
        return subprocess.run(
            [str(part) for part in command],
            cwd=cwd,
            check=check,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            stderr=subprocess.PIPE if capture else None,
        )
    except subprocess.CalledProcessError as error:
        if capture:
            print(error.stdout or "", end="", file=sys.stderr)
            print(error.stderr or "", end="", file=sys.stderr)
        raise


def output(command: Sequence[str], *, cwd: Path | None = None) -> str:
    result = run(command, cwd=cwd, capture=True)
    return result.stdout.strip()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repository_root() -> Path:
    return Path(__file__).resolve().parents[1]


def ensure_checkout(root: Path, patch: Path) -> Path:
    patch_hash = sha256_file(patch)
    checkout = (
        root
        / "target"
        / "ironpress-checkouts"
        / f"{IRONPRESS_COMMIT[:12]}-{patch_hash[:12]}"
    )
    if not checkout.exists():
        checkout.parent.mkdir(parents=True, exist_ok=True)
        clone_command = ["git", "clone", "--filter=blob:none", "--no-checkout"]
        local_reference = root / "target" / "ironpress-upstream"
        if (local_reference / ".git").is_dir():
            clone_command.extend(
                ["--reference-if-able", str(local_reference), "--dissociate"]
            )
        clone_command.extend([IRONPRESS_REPOSITORY, str(checkout)])
        run(clone_command)
        run(["git", "checkout", "--detach", IRONPRESS_COMMIT], cwd=checkout)
        run(["git", "apply", "--check", str(patch)], cwd=checkout)
        run(["git", "apply", str(patch)], cwd=checkout)

    head = output(["git", "rev-parse", "HEAD"], cwd=checkout)
    if head != IRONPRESS_COMMIT:
        raise RuntimeError(f"unexpected IronPress checkout HEAD: {head}")
    run(["git", "apply", "--check", "--reverse", str(patch)], cwd=checkout)
    changed = output(["git", "diff", "HEAD", "--name-only"], cwd=checkout).splitlines()
    if changed != ["tests/parity_support/render.rs"]:
        raise RuntimeError(f"unexpected IronPress checkout changes: {changed}")
    return checkout


def ensure_image(root: Path, *, rebuild: bool) -> None:
    present = subprocess.run(
        ["docker", "image", "inspect", IMAGE_TAG],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    ).returncode == 0
    if rebuild or not present:
        run(
            [
                "docker",
                "build",
                "--file",
                str(root / "tools" / "ironpress_parity.Dockerfile"),
                "--tag",
                IMAGE_TAG,
                str(root),
            ]
        )


def volume_is_empty(volume: str) -> bool:
    probe = subprocess.run(
        [
            "docker",
            "run",
            "--rm",
            "--volume",
            f"{volume}:/ironpress",
            IMAGE_TAG,
            "sh",
            "-lc",
            "test -z \"$(find /ironpress -mindepth 1 -maxdepth 1 -print -quit)\"",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return probe.returncode == 0


def ensure_source_volume(checkout: Path, patch_hash: str) -> str:
    volume = f"fullbleed-ironpress-source-{IRONPRESS_COMMIT[:12]}-{patch_hash[:12]}"
    created = subprocess.run(
        ["docker", "volume", "inspect", volume],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    ).returncode != 0
    if created:
        run(["docker", "volume", "create", volume], capture=True)
    if volume_is_empty(volume):
        run(
            [
                "docker",
                "run",
                "--rm",
                "--volume",
                f"{checkout.resolve()}:/seed:ro",
                "--volume",
                f"{volume}:/ironpress",
                IMAGE_TAG,
                "sh",
                "-lc",
                "cp -a /seed/. /ironpress/",
            ]
        )

    head = output(
        [
            "docker",
            "run",
            "--rm",
            "--volume",
            f"{volume}:/ironpress:ro",
            IMAGE_TAG,
            "git",
            "-c",
            "safe.directory=/ironpress",
            "-C",
            "/ironpress",
            "rev-parse",
            "HEAD",
        ]
    )
    if head != IRONPRESS_COMMIT:
        raise RuntimeError(
            f"source volume {volume} is not the pinned IronPress checkout: {head}"
        )
    # A reused volume can outlive its source checkout. Verify its actual
    # tracked inputs too; generated reports are the only additional changes
    # allowed after a previous run.
    probe = ["docker", "run", "--rm", "--volume", f"{volume}:/ironpress:ro"]
    changed = output([
        *probe, IMAGE_TAG, "git", "-c", "safe.directory=/ironpress",
        "-C", "/ironpress", "diff", "HEAD", "--name-only",
    ]).splitlines()
    generated = ("tests/parity/report.json", "tests/parity/REPORT.md")
    generated_dirs = tuple(f"tests/parity/{name}/" for name in ("reports", "refs", "out", "diffs", "pdfs"))
    unexpected = [
        name for name in changed
        if name != "tests/parity_support/render.rs"
        and name not in generated and not name.startswith(generated_dirs)
    ]
    if unexpected:
        raise RuntimeError(f"source volume has modified upstream inputs: {unexpected}")
    patch = repository_root() / "tools" / "ironpress_fullbleed.patch"
    run([
        *probe, "--volume", f"{patch.resolve()}:/candidate.patch:ro",
        IMAGE_TAG, "git", "-c", "safe.directory=/ironpress",
        "-C", "/ironpress", "apply", "--check", "--reverse", "/candidate.patch",
    ])
    # This upstream library revision does not commit Cargo.lock. Resolve its
    # unchanged manifest once with Rust 1.97 and retain that integration lock
    # in Fullbleed so subsequent runs use the same comparator dependencies.
    cargo_lock = repository_root() / "tools" / "ironpress.Cargo.lock"
    run([
        "docker", "run", "--rm", "--volume", f"{volume}:/ironpress",
        "--volume", f"{cargo_lock.resolve()}:/pinned-Cargo.lock:ro",
        IMAGE_TAG, "cp", "/pinned-Cargo.lock", "/ironpress/Cargo.lock",
    ])
    return volume


def discover_wheel(root: Path, requested: Path | None) -> Path:
    if requested is not None:
        wheel = requested.resolve(strict=True)
    else:
        candidates: list[Path] = []
        for pattern in LINUX_WHEEL_PATTERNS:
            candidates.extend((root / "target" / "ironpress-wheel").glob(pattern))
            candidates.extend((root / "dist").glob(f"**/{pattern}"))
        if not candidates:
            raise RuntimeError(
                "no Linux x86-64 FullBleed wheel found; pass --wheel after building one"
            )
        wheel = max(candidates, key=lambda path: path.stat().st_mtime).resolve()
    try:
        wheel.relative_to(root)
    except ValueError as error:
        raise RuntimeError("--wheel must be inside the FullBleed repository") from error
    if not any(fnmatch.fnmatchcase(wheel.name, pattern) for pattern in LINUX_WHEEL_PATTERNS):
        raise RuntimeError(f"not a compatible Linux x86-64 FullBleed wheel: {wheel.name}")
    wheel_identity(wheel)
    return wheel


def wheel_identity(wheel: Path) -> dict[str, str]:
    """Check package identity as well as the wheel's Linux/abi3 filename."""
    match = re.fullmatch(
        r"fullbleed-(?P<version>[A-Za-z0-9_.!+]+)-cp310-abi3-"
        r"(?P<platforms>[^/\\]+)\.whl", wheel.name
    )
    if match is None or not all(
        re.fullmatch(r"(?:linux|manylinux(?:\d+|_\d+_\d+))_x86_64", tag)
        for tag in match["platforms"].split(".")
    ):
        raise RuntimeError(f"not a compatible Linux x86-64 FullBleed wheel: {wheel.name}")
    with zipfile.ZipFile(wheel) as archive:
        metadata = [name for name in archive.namelist() if name.endswith(".dist-info/METADATA")]
        if len(metadata) != 1:
            raise RuntimeError("wheel must contain exactly one package METADATA file")
        package = BytesParser().parsebytes(archive.read(metadata[0]))
    if package.get("Name", "").lower() != "fullbleed" or package.get("Version") != match["version"]:
        raise RuntimeError("wheel filename and FullBleed package metadata disagree")
    return {"filename": wheel.name, "version": match["version"], "sha256": sha256_file(wheel)}


def check_full_report(destination: Path, invocation: str) -> dict[str, object]:
    """Reject stale, incomplete, or inconsistent evidence, even after exit zero."""
    problems: list[str] = []
    try:
        report_path = destination / "report.json"
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if report.get("invocation_id") != invocation:
            problems.append("report invocation does not match this run")
        if report.get("run_complete") is not True:
            problems.append("upstream run is incomplete")
        fixtures = [
            fixture
            for category in report["categories"]
            for feature in category["features"]
            for fixture in feature["fixtures"]
        ]
        counts = Counter(fixture["status"] for fixture in fixtures)
        identities = {(fixture["category"], fixture["id"]) for fixture in fixtures}
        overall = report["overall"]
        if len(fixtures) != CORPUS_FIXTURES or len(identities) != CORPUS_FIXTURES:
            problems.append("report does not contain the complete unique pinned corpus")
        expected = {
            "pass": counts["PASS"], "fail": counts["FAIL"],
            "reference_disputed": counts["REFERENCE-DISPUTED"], "total": len(fixtures),
        }
        if set(counts) - {"PASS", "FAIL", "REFERENCE-DISPUTED"} or any(
            overall.get(key) != value for key, value in expected.items()
        ):
            problems.append("summary counts do not match fixture verdicts")
        digest = sha256_file(report_path)
        markers = {
            "REPORT.md": (
                f"<!-- parity-invocation-id: {invocation} -->",
                f"<!-- parity-report-json-sha256: {digest} -->",
            ),
            "reports/index.html": (
                f'<meta name="parity-invocation-id" content="{invocation}">',
                f'<meta name="parity-report-json-sha256" content="{digest}">',
            ),
        }
        for name, required in markers.items():
            content = (destination / name).read_text(encoding="utf-8")
            if not all(marker in content for marker in required):
                problems.append(f"{name} is not bound to the same report")
        return {
            "verified_complete": not problems,
            "overall": overall,
            "gate_failure": report.get("gate_failure"),
            "report_sha256": digest,
            "problems": problems,
        }
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        problems.append(f"cannot verify full report: {error}")
        return {"verified_complete": False, "problems": problems}


def write_run_manifest(destination: Path, provenance: dict[str, object]) -> None:
    """Retain identities and an inventory without rewriting upstream reports."""
    (destination / "fullbleed-run.json").write_text(
        json.dumps(provenance, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    files = {
        path.relative_to(destination).as_posix(): {
            "bytes": path.stat().st_size, "sha256": sha256_file(path),
        }
        for path in sorted(destination.rglob("*"))
        if path.is_file() and path != destination / "manifest.json"
    }
    (destination / "manifest.json").write_text(
        json.dumps({"schema": "fullbleed.evidence_manifest.v1", "files": files}, indent=2) + "\n",
        encoding="utf-8",
    )


def latest_diagnostic_path(volume: str) -> str:
    return output(
        [
            "docker",
            "run",
            "--rm",
            "--volume",
            f"{volume}:/ironpress:ro",
            IMAGE_TAG,
            "sh",
            "-lc",
            "ls -1dt /ironpress/target/parity-diagnostics/run-* 2>/dev/null | head -n 1",
        ]
    )


def copy_evidence(
    volume: str, destination: Path, *, diagnostic_path: str | None = None,
    keep_pdfs: bool = False,
) -> list[str]:
    destination.mkdir(parents=True, exist_ok=True)
    problems: list[str] = []
    container = output(
        [
            "docker",
            "create",
            "--volume",
            f"{volume}:/ironpress:ro",
            IMAGE_TAG,
            "true",
        ]
    )
    try:
        license_copy = run([
            "docker", "cp", f"{container}:/ironpress/LICENSE",
            str(destination / "IRONPRESS-LICENSE"),
        ], check=False)
        if license_copy.returncode:
            problems.append("could not retain upstream LICENSE")
        lock_copy = run([
            "docker", "cp", f"{container}:/ironpress/Cargo.lock",
            str(destination / "Cargo.lock"),
        ], check=False)
        if lock_copy.returncode:
            problems.append("could not retain comparator Cargo.lock")
        if diagnostic_path:
            run(
                [
                    "docker",
                    "cp",
                    f"{container}:{diagnostic_path}/.",
                    str(destination),
                ],
            )
        else:
            paths = [
                "tests/parity/report.json",
                "tests/parity/REPORT.md",
                "tests/parity/reports",
                "tests/parity/refs",
                "tests/parity/out",
                "tests/parity/diffs",
            ]
            if keep_pdfs:
                paths.append("tests/parity/pdfs")
            for relative in paths:
                copied = run(
                    [
                        "docker",
                        "cp",
                        f"{container}:/ironpress/{relative}",
                        str(destination),
                    ], check=False,
                )
                if copied.returncode:
                    problems.append(f"could not retain {relative}")
    finally:
        run(["docker", "rm", container], capture=True, check=False)
    return problems


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--only", help="IronPress PARITY_ONLY diagnostic filter")
    result.add_argument("--wheel", type=Path, help="Linux x86-64 FullBleed wheel")
    result.add_argument("--threads", type=int, default=1, help="threads per adapter process")
    result.add_argument("--keep-pdfs", action="store_true")
    result.add_argument("--rebuild-image", action="store_true")
    result.add_argument("--evidence-dir", type=Path)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    if arguments.threads < 1:
        raise SystemExit("--threads must be at least 1")
    if shutil.which("docker") is None or shutil.which("git") is None:
        raise SystemExit("git and docker are required")

    root = repository_root()
    # Reject invalid inputs before building an image or populating caches.
    wheel = discover_wheel(root, arguments.wheel)
    invocation = f"fullbleed-{uuid.uuid4().hex}"
    evidence = arguments.evidence_dir or root / "target" / "ironpress-evidence" / invocation
    if evidence.exists() and (not evidence.is_dir() or any(evidence.iterdir())):
        raise RuntimeError(f"evidence directory must be empty: {evidence}")
    evidence.mkdir(parents=True, exist_ok=True)
    patch = root / "tools" / "ironpress_fullbleed.patch"
    patch_hash = sha256_file(patch)
    provenance: dict[str, object] = {
        "schema": "fullbleed.ironpress_run.v1",
        "invocation_id": invocation,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "scope": "filtered_diagnostic" if arguments.only else "full_corpus",
        "filter": arguments.only,
        "threads_per_adapter": arguments.threads,
        "wheel": wheel_identity(wheel),
        "runner_commit": output(["git", "rev-parse", "HEAD"], cwd=root),
        "runner_sha256": sha256_file(Path(__file__)),
        "upstream_repository": IRONPRESS_REPOSITORY,
        "upstream_commit": IRONPRESS_COMMIT,
        "patch_sha256": patch_hash,
        "adapter_sha256": sha256_file(root / "tools" / "ironpress_fullbleed_adapter.py"),
        "dockerfile_sha256": sha256_file(root / "tools" / "ironpress_parity.Dockerfile"),
        "cargo_lock_sha256": sha256_file(root / "tools" / "ironpress.Cargo.lock"),
        "gate_passed": False,
    }
    write_run_manifest(evidence, provenance)
    ensure_image(root, rebuild=arguments.rebuild_image)
    provenance["image_id"] = output(["docker", "image", "inspect", "--format", "{{.Id}}", IMAGE_TAG])
    checkout = ensure_checkout(root, patch)
    source_volume = ensure_source_volume(checkout, patch_hash)
    wheel_in_container = "/fullbleed/" + wheel.relative_to(root).as_posix()

    command = [
        "docker",
        "run",
        "--rm",
        "--env",
        "CARGO_TARGET_DIR=/cargo-target",
        "--env",
        "CARGO_TERM_COLOR=never",
        "--env",
        "FULLBLEED_PARITY_ADAPTER=/fullbleed/tools/ironpress_fullbleed_adapter.py",
        "--env",
        "FULLBLEED_PARITY_PYTHON=/venv/bin/python",
        "--env",
        f"FULLBLEED_THREADS={arguments.threads}",
        "--env",
        "FONTCONFIG_FILE=/ironpress/tests/parity/fonts/fonts.conf",
        "--env",
        "PARITY_PDFTOPPM=/usr/bin/pdftoppm",
        "--volume",
        f"{root.resolve()}:/fullbleed:ro",
        "--volume",
        f"{source_volume}:/ironpress",
        "--volume",
        "fullbleed-ironpress-cargo-target:/cargo-target",
        "--volume",
        "fullbleed-ironpress-cargo-registry:/usr/local/cargo/registry",
        "--workdir",
        "/ironpress",
    ]
    if arguments.only:
        command.extend(["--env", f"PARITY_ONLY={arguments.only}"])
    else:
        command.extend(["--env", f"PARITY_INVOCATION_ID={invocation}"])
    if arguments.keep_pdfs:
        command.extend(["--env", "PARITY_KEEP_PDFS=1"])
    command.extend(
        [
            IMAGE_TAG,
            "sh",
            "-lc",
            f"python -m pip install --disable-pip-version-check --no-index "
            f"--force-reinstall {shlex.quote(wheel_in_container)} >/dev/null && "
            "cargo test --locked --test feature_parity -- --ignored --nocapture --exact feature_parity",
        ]
    )

    started = time.perf_counter()
    result = run(command, check=False)
    elapsed = time.perf_counter() - started
    diagnostic_path = latest_diagnostic_path(source_volume) if arguments.only else None
    if arguments.only and not diagnostic_path:
        raise RuntimeError("IronPress produced no filtered diagnostic evidence directory")
    copy_problems = copy_evidence(
        source_volume, evidence, diagnostic_path=diagnostic_path, keep_pdfs=arguments.keep_pdfs,
    )
    if (evidence / "Cargo.lock").is_file() and sha256_file(evidence / "Cargo.lock") != provenance["cargo_lock_sha256"]:
        copy_problems.append("exported comparator Cargo.lock differs from the pinned input")
    checked = check_full_report(evidence, invocation) if not arguments.only else {
        "verified_complete": False, "problems": ["filtered diagnostics cannot satisfy the full-corpus gate"],
    }
    provenance.update({
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "elapsed_seconds": elapsed,
        "process_exit_code": result.returncode,
        "report_check": checked,
        "copy_problems": copy_problems,
        "gate_passed": (
            result.returncode == 0 and checked["verified_complete"]
            and not checked.get("gate_failure") and not copy_problems
            and checked.get("overall", {}).get("fail") == 0
        ),
    })
    write_run_manifest(evidence, provenance)
    print(f"IronPress elapsed: {elapsed:.3f}s", file=sys.stderr)
    print(f"Evidence: {evidence.resolve()}", file=sys.stderr)
    return result.returncode or (0 if provenance["gate_passed"] else 1)


if __name__ == "__main__":
    raise SystemExit(main())
