#!/usr/bin/env python3
"""Run the pinned IronPress parity corpus through FullBleed.

Only IronPress's candidate-renderer boundary is patched. Fixtures, manifests,
browser PDF oracles, rasterization, comparison, and reporting remain upstream.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Sequence


IRONPRESS_REPOSITORY = "https://github.com/gastongouron/ironpress.git"
IRONPRESS_COMMIT = "0d1e53b6d8174d0a5059a8696c24e62759381f6d"
IMAGE_TAG = "fullbleed-ironpress-parity:0d1e53b6"
LINUX_WHEEL_PATTERNS = (
    "fullbleed-2.2.2-cp310-abi3-manylinux*_x86_64.whl",
    "fullbleed-2.2.2-cp310-abi3-linux_x86_64.whl",
)


def run(
    command: Sequence[str],
    *,
    cwd: Path | None = None,
    capture: bool = False,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    printable = subprocess.list2cmdline([str(part) for part in command])
    print(f"+ {printable}", file=sys.stderr, flush=True)
    return subprocess.run(
        [str(part) for part in command],
        cwd=cwd,
        check=check,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )


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
    changed = output(["git", "diff", "--name-only"], cwd=checkout).splitlines()
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
    return wheel


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
    volume: str, destination: Path, *, diagnostic_path: str | None = None
) -> None:
    destination.mkdir(parents=True, exist_ok=True)
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
            for relative in (
                "tests/parity/report.json",
                "tests/parity/REPORT.md",
                "tests/parity/reports",
            ):
                run(
                    [
                        "docker",
                        "cp",
                        f"{container}:/ironpress/{relative}",
                        str(destination),
                    ]
                )
    finally:
        run(["docker", "rm", container], capture=True, check=False)


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
    patch = root / "tools" / "ironpress_fullbleed.patch"
    patch_hash = sha256_file(patch)
    ensure_image(root, rebuild=arguments.rebuild_image)
    checkout = ensure_checkout(root, patch)
    source_volume = ensure_source_volume(checkout, patch_hash)
    wheel = discover_wheel(root, arguments.wheel)
    wheel_in_container = "/fullbleed/" + wheel.relative_to(root).as_posix()

    invocation = f"fullbleed-{int(time.time())}-{os.getpid()}"
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
            f"--force-reinstall {wheel_in_container} >/dev/null && "
            "cargo test --test feature_parity -- --ignored --nocapture --exact feature_parity",
        ]
    )

    started = time.perf_counter()
    result = run(command, check=False)
    elapsed = time.perf_counter() - started
    evidence = arguments.evidence_dir or (
        root
        / "target"
        / "ironpress-evidence"
        / (arguments.only or invocation).replace("/", "-")
    )
    diagnostic_path = latest_diagnostic_path(source_volume) if arguments.only else None
    if arguments.only and not diagnostic_path:
        raise RuntimeError("IronPress produced no filtered diagnostic evidence directory")
    copy_evidence(source_volume, evidence, diagnostic_path=diagnostic_path)
    print(f"IronPress elapsed: {elapsed:.3f}s", file=sys.stderr)
    print(f"Evidence: {evidence.resolve()}", file=sys.stderr)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
