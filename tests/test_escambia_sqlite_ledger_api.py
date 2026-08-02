"""Regression coverage for the historical, never-shipped Escambia API extra."""

from __future__ import annotations

import importlib.util
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
BACKEND_PATH = REPO_ROOT / "build_backend" / "fullbleed_build_backend.py"


def _load_backend():
    spec = importlib.util.spec_from_file_location(
        "fullbleed_build_backend_api_extra_test", BACKEND_PATH
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_api_extra_remains_valid_without_installing_an_http_stack() -> None:
    backend = _load_backend()
    project = backend._load_pyproject()["project"]
    assert project["optional-dependencies"]["api"] == []

    metadata_headers = (
        backend._metadata_bytes().decode("utf-8").split("\n\n", 1)[0].splitlines()
    )
    assert "Provides-Extra: api" in metadata_headers
    assert not any(header.startswith("Requires-Dist:") for header in metadata_headers)


def test_ignored_escambia_workspace_is_not_part_of_python_distributions() -> None:
    backend = _load_backend()
    assert not any(name.startswith("_escambia/") for name in backend._python_entries())
    assert not any(
        path.relative_to(REPO_ROOT).as_posix().startswith("_escambia/")
        for path in backend._sdist_files()
    )
