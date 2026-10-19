# SPDX-License-Identifier: MIT
"""Fixed local-demo assets shared by the three framework adapters."""
from importlib.metadata import version
from pathlib import Path

STATIC = Path(__file__).resolve().parent / "static"
DEMO_HEADERS = {"Cache-Control": "no-store"}


def page_html() -> str:
    return (STATIC / "index.html").read_text(encoding="utf-8")


def preview_path() -> Path:
    """Use a reviewed version-specific sample when the source alpha differs."""
    candidate = STATIC / f"invoice-{version('fullbleed')}.png"
    return candidate if candidate.is_file() else STATIC / "invoice.png"


def preview_bytes() -> bytes:
    return preview_path().read_bytes()
