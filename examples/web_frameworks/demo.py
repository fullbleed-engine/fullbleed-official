# SPDX-License-Identifier: MIT
"""Fixed local-demo assets shared by the three framework adapters."""
from pathlib import Path

STATIC = Path(__file__).resolve().parent / "static"
DEMO_HEADERS = {"Cache-Control": "no-store"}


def page_html() -> str:
    return (STATIC / "index.html").read_text(encoding="utf-8")


def preview_bytes() -> bytes:
    return (STATIC / "invoice.png").read_bytes()
