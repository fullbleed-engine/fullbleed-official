#!/usr/bin/env python
"""Render with an unmodified native main-thread stack, outside Rust's test harness."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time


def windows_stack_reserve(path: Path) -> int:
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError("Expected a Windows PE executable")
    pe = int.from_bytes(data[0x3C:0x40], "little")
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE signature")
    header = pe + 24
    magic = int.from_bytes(data[header:header + 2], "little")
    if magic == 0x20B:
        offset, size = 72, 8
    elif magic == 0x10B:
        offset, size = 72, 4
    else:
        raise ValueError(f"Unknown PE optional header: {magic:#x}")
    return int.from_bytes(data[header + offset:header + offset + size], "little")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/rust-stack-smoke"))
    parser.add_argument("--profile", choices=("dev", "release"), default="dev")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    project = out / "consumer"
    (project / "src").mkdir(parents=True)
    shutil.copyfile(root / "tools/rust_stack_consumer.rs", project / "src/main.rs")
    shutil.copyfile(root / "Cargo.lock", project / "Cargo.lock")
    (project / "Cargo.toml").write_text(
        '[package]\nname = "fullbleed-default-stack-smoke"\n'
        'version = "0.0.0"\nedition = "2024"\npublish = false\n'
        '[dependencies]\nfullbleed = { path = ' + json.dumps(root.as_posix()) + ' }\n',
        encoding="utf-8",
    )
    env = dict(os.environ)
    # A global CI test-harness override must not conceal this regression.
    for name in ("RUST_MIN_STACK", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(name, None)
    target = Path(env.get("CARGO_TARGET_DIR", str(root / "target"))).resolve()
    env["CARGO_TARGET_DIR"] = str(target)
    record = {
        "ok": False, "platform": platform.platform(), "profile": args.profile,
        "worker_thread_override": False, "RUST_MIN_STACK_present": False,
        "scope": "Finite rendering regressions on the native main thread; no arbitrary-depth guarantee.",
    }
    start = time.monotonic()
    try:
        record["rustc"] = subprocess.check_output(["rustc", "-Vv"], text=True).strip()
        command = ["cargo", "build", "--offline", "--manifest-path", str(project / "Cargo.toml")]
        if args.profile == "release":
            command.append("--release")
        with (out / "build.log").open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, cwd=root, env=env, stdout=stream,
                                    stderr=subprocess.STDOUT, timeout=1200)
        if result.returncode:
            raise RuntimeError(f"Build exited {result.returncode}; see build.log")
        suffix = ".exe" if os.name == "nt" else ""
        binary = target / ("debug" if args.profile == "dev" else "release") / ("fullbleed-default-stack-smoke" + suffix)
        record["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        if os.name == "nt":
            reserve = windows_stack_reserve(binary)
            record["main_stack_reserve_bytes"] = reserve
            if reserve != 1024 * 1024:
                raise RuntimeError(f"Expected Windows default 1 MiB main stack, got {reserve}")
        pdfs = out / "pdfs"
        pdfs.mkdir()
        with (out / "run.log").open("w", encoding="utf-8") as stream:
            result = subprocess.run(
                [str(binary), str(root / "python/fullbleed_assets/fonts/Inter-Variable.ttf"), str(pdfs)],
                cwd=root, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=900,
            )
        record["exit_code"] = result.returncode
        if result.returncode:
            raise RuntimeError(f"Consumer exited {result.returncode}; see run.log")
        record["passed_cases"] = [line.removeprefix("PASS ") for line in
                                  (out / "run.log").read_text(encoding="utf-8").splitlines()
                                  if line.startswith("PASS ")]
        files = sorted(pdfs.glob("*.pdf"))
        if len(record["passed_cases"]) != 9 or len(files) != 33:
            raise RuntimeError("Consumer did not complete all nine cases and 33 PDFs")
        record["pdfs"] = [{"file": p.name, "bytes": p.stat().st_size,
                           "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in files]
        record["ok"] = True
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        record["error"] = str(error)
    record["elapsed_seconds"] = round(time.monotonic() - start, 3)
    (out / "verification.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in record.items() if key != "pdfs"}, indent=2))
    return 0 if record["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
