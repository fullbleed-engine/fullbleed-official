#!/usr/bin/env python3
"""Link musllinux extensions without a runtime dependency on libgcc_s."""

from __future__ import annotations

import os
import subprocess
import sys
from collections.abc import Sequence


def rewrite_linker_arguments(arguments: Sequence[str]) -> list[str]:
    """Replace Rust's dynamic GCC unwinder with the toolchain's static archives."""

    rewritten: list[str] = []
    replacements = 0
    for argument in arguments:
        if argument == "-lgcc_s":
            rewritten.extend(
                ("-Wl,-Bstatic", "-lgcc_eh", "-lgcc", "-Wl,-Bdynamic")
            )
            replacements += 1
        else:
            rewritten.append(argument)
    if replacements != 1:
        raise ValueError(
            f"expected exactly one -lgcc_s argument, found {replacements}"
        )
    return rewritten


def main() -> int:
    real_linker = os.environ.get("FULLBLEED_MUSL_REAL_LINKER")
    if not real_linker:
        print(
            "fullbleed-build: FULLBLEED_MUSL_REAL_LINKER is not set",
            file=sys.stderr,
        )
        return 1
    try:
        arguments = rewrite_linker_arguments(sys.argv[1:])
    except ValueError as error:
        print(f"fullbleed-build: {error}", file=sys.stderr)
        return 1
    return subprocess.run([real_linker, *arguments], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
