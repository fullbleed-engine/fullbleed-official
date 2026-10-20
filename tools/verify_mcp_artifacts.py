#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Independently check the retained documents from smoke_mcp_stdio.py."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path

from PIL import Image
from pypdf import PdfReader


def verify(root: Path) -> dict:
    root = root.resolve()
    preview = root / "preview" / "preview.pdf"
    vdp = root / "output" / "vdp.pdf"
    pages = sorted((root / "preview" / "pages").glob("*.png"))
    checks = []

    def check(name: str, passed: bool) -> None:
        if not passed:
            raise AssertionError(name)
        checks.append(name)

    document = PdfReader(preview)
    check("preview PDF has one page", len(document.pages) == 1)
    text = document.pages[0].extract_text() or ""
    check("preview PDF text is readable", "MCP Preview" in text and text.count("MCP-SMOKE-001") == 1)
    check("preview has one PNG", len(pages) == 1)
    with Image.open(pages[0]) as image:
        rgba = image.convert("RGBA")
        ink = sum(1 for red, green, blue, alpha in rgba.get_flattened_data()
                  if alpha > 0 and min(red, green, blue) < 245)
        dimensions = list(image.size)
    check("standard-font preview contains visible ink", ink > 100)
    records = PdfReader(vdp)
    check("VDP PDF has three pages", len(records.pages) == 3)
    identifiers = [f"MCP-VDP-{index:03}" for index in range(1, 4)]
    for index, page in enumerate(records.pages):
        text = page.extract_text() or ""
        check(f"VDP page {index + 1} contains only its expected record",
              text.count(identifiers[index]) == 1
              and all(other not in text for other in identifiers if other != identifiers[index]))
    return {
        "schema": "fullbleed.mcp_artifact_check.v1", "ok": True,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "readers": {"pypdf": version("pypdf"), "pillow": version("pillow")},
        "checks": checks, "preview": {"dimensions": dimensions, "ink_pixels": ink},
        "files": {path.relative_to(root).as_posix(): {
            "sha256": sha256(path.read_bytes()).hexdigest(), "bytes": path.stat().st_size,
        } for path in [preview, *pages, vdp]},
        "scope": "Synthetic MCP smoke fixtures: extracted text, page counts, record order and a nonblank standard-font preview. Not a conformance audit.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.directory)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
