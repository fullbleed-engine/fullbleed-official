from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
LINKER_PATH = REPO_ROOT / "tools" / "fullbleed_musl_linker.py"


def _load_linker_module():
    spec = importlib.util.spec_from_file_location(
        "fullbleed_musl_linker_test", LINKER_PATH
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_rewrite_linker_arguments_static_links_the_gcc_unwinder() -> None:
    linker = _load_linker_module()

    assert linker.rewrite_linker_arguments(
        ["input.o", "-Wl,-Bdynamic", "-lgcc_s", "-lc", "-shared"]
    ) == [
        "input.o",
        "-Wl,-Bdynamic",
        "-Wl,-Bstatic",
        "-lgcc_eh",
        "-lgcc",
        "-Wl,-Bdynamic",
        "-lc",
        "-shared",
    ]


@pytest.mark.parametrize("arguments", [[], ["-lgcc_s", "-lgcc_s"]])
def test_rewrite_linker_arguments_requires_one_dynamic_unwinder(
    arguments: list[str],
) -> None:
    linker = _load_linker_module()

    with pytest.raises(ValueError, match="exactly one -lgcc_s"):
        linker.rewrite_linker_arguments(arguments)
