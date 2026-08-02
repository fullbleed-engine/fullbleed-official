from __future__ import annotations

import importlib.machinery
from pathlib import Path

import conftest as test_configuration


def test_installed_native_resolution_excludes_source_and_finds_platform_suffix(
    tmp_path: Path,
) -> None:
    installed = tmp_path / "site-packages" / "fullbleed"
    installed.mkdir(parents=True)
    candidate = installed / f"_fullbleed{importlib.machinery.EXTENSION_SUFFIXES[0]}"
    candidate.write_bytes(b"test extension placeholder")

    observed = test_configuration._installed_fullbleed_native_candidates(
        [str(test_configuration.PYTHON_SRC), str(tmp_path / "site-packages")]
    )

    assert observed == [candidate]

    class EditableFinder:
        _fullbleed_editable_finder = True

    ordinary_finder = object()
    assert test_configuration._without_fullbleed_editable_finders(
        [ordinary_finder, EditableFinder()]
    ) == [ordinary_finder]
