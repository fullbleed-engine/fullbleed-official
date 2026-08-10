from __future__ import annotations

import json
from pathlib import Path

from tools import generate_mcp_server_json


REPO_ROOT = Path(__file__).resolve().parents[1]
PACKAGE_ROOT = REPO_ROOT / "packages" / "fullbleed-mcp"


def test_mcp_registry_metadata_is_generated_from_package_metadata() -> None:
    expected = generate_mcp_server_json.build_server_json()
    observed = json.loads((PACKAGE_ROOT / "server.json").read_text(encoding="utf-8"))
    assert observed == expected
    assert observed["$schema"].endswith("/2025-12-11/server.schema.json")
    assert observed["name"] == "io.github.fullbleed-engine/fullbleed-mcp"
    assert observed["version"] == "0.1.0"
    assert observed["packages"] == [
        {
            "registryType": "pypi",
            "identifier": "fullbleed-mcp",
            "version": "0.1.0",
            "transport": {"type": "stdio"},
        }
    ]
    assert len(observed["description"]) <= 100


def test_mcp_pypi_readme_contains_registry_ownership_marker() -> None:
    readme = (PACKAGE_ROOT / "README.md").read_text(encoding="utf-8")
    assert "mcp-name: io.github.fullbleed-engine/fullbleed-mcp" in readme
    assert "pip install fullbleed-mcp" in readme
    assert "screenshots" in readme or "screenshot" in readme


def test_mcp_distribution_wraps_runtime_without_duplicate_renderer() -> None:
    module = (
        PACKAGE_ROOT / "src" / "fullbleed_mcp" / "__init__.py"
    ).read_text(encoding="utf-8")
    metadata = (PACKAGE_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    core_metadata = (REPO_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    assert "from fullbleed_cli.mcp import" in module
    assert 'dependencies = ["fullbleed>=2.3.0,<3"]' in metadata
    assert "fullbleed-mcp" not in core_metadata.split("[project.scripts]", 1)[1].split(
        "[project.urls]", 1
    )[0]
