#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify border-image source overlap and destination widths in installed output.

Expected interior colors follow CSS Backgrounds 3, sections 5.2 and 5.3:
https://www.w3.org/TR/css-backgrounds-3/#border-image-slice
Source slices can overlap; destination widths are proportionally reduced.
No browser or imaging dependency is required for the native checks. --pdfium
additionally checks the actual PDFs with the optional pypdfium2 test dependency.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
from importlib import metadata
import json
from pathlib import Path
import runpy
import struct
import urllib.parse
import zlib

import fullbleed

decode_png = runpy.run_path(str(Path(__file__).with_name("run_css_fixture_suite.py")))["_decode_png_rgba_rows"]
WHITE, RED, BLUE, CYAN = [255, 255, 255], [255, 0, 0], [0, 0, 255], [17, 133, 171]
HORIZONTAL = "linear-gradient(90deg,red 0%,red 50%,blue 50%,blue 100%)"
VERTICAL = "linear-gradient(180deg,red 0%,red 20%,blue 20%,blue 80%,red 80%,red 100%)"
SOLID = "linear-gradient(#1185ab,#1185ab)"
HTML = "<!doctype html><html><body><div></div></body></html>"
BASE_CSS = """@page { size: 160px 120px; margin: 0; }
html, body { margin: 0; padding: 0; background: white; }
div { position: absolute; left: 20px; top: 20px; box-sizing: border-box;
      width: 120px; height: 80px; border: 10px solid transparent; }
"""


def raster_source() -> str:
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    row = b"\0" + bytes(RED + [255]) * 5 + bytes(BLUE + [255]) * 5
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 10, 10, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(row * 10)) + chunk(b"IEND", b""))
    return 'url("data:image/png;base64,' + base64.b64encode(png).decode("ascii") + '")'


def svg_source() -> str:
    svg = '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="60" height="80" fill="red"/><rect x="60" width="60" height="80" fill="blue"/></svg>'
    return 'url("data:image/svg+xml,' + urllib.parse.quote(svg, safe="") + '")'


# Coordinates are inside the 120 x 80 CSS-pixel border box, away from edges.
# These assert visible regions, not the renderer's internal slice arithmetic.
CASES = [
    ("overlapping-gradient", HORIZONTAL, "80%", "90px", False,
     [(35, 20, BLUE), (85, 20, RED), (60, 20, WHITE), (35, 60, BLUE)]),
    ("overlapping-raster", raster_source(), "80%", "90px", False,
     [(35, 20, BLUE), (85, 20, RED), (60, 20, WHITE)]),
    ("overlapping-svg", svg_source(), "80%", "90px", False,
     [(35, 20, BLUE), (85, 20, RED), (60, 20, WHITE)]),
    ("overlap-auto-width", HORIZONTAL, "80%", "auto", False,
     [(45, 20, BLUE), (75, 20, RED)]),
    ("clamp-percent", HORIZONTAL, "200%", "90px", False,
     [(30, 20, BLUE), (90, 20, RED), (60, 20, WHITE)]),
    ("clamp-number", HORIZONTAL, "1000", "90px", False,
     [(30, 20, BLUE), (90, 20, RED), (60, 20, WHITE)]),
    ("destination-width-reduction", VERTICAL, "20%", "20px 90px", False,
     [(10, 8, RED), (10, 17, BLUE), (110, 63, BLUE), (110, 72, RED)]),
]
for fill in [False, True]:
    suffix = "-fill" if fill else "-unfilled"
    CASES.extend([
        ("uniform-overlap" + suffix, SOLID, "80%", "10px", fill,
         [(5, 5, CYAN), (115, 75, CYAN), (60, 5, WHITE), (5, 40, WHITE), (60, 40, WHITE)]),
        ("uniform-touching" + suffix, SOLID, "50%", "10px", fill,
         [(5, 5, CYAN), (60, 5, WHITE), (5, 40, WHITE), (60, 40, WHITE)]),
        ("x-overlap" + suffix, SOLID, "25% 80%", "10px", fill,
         [(5, 5, CYAN), (60, 5, WHITE), (60, 75, WHITE), (5, 40, CYAN), (60, 40, WHITE)]),
        ("y-overlap" + suffix, SOLID, "80% 25%", "10px", fill,
         [(5, 5, CYAN), (60, 5, CYAN), (5, 40, WHITE), (115, 40, WHITE), (60, 40, WHITE)]),
        ("nonoverlap-control" + suffix, SOLID, "25%", "10px", fill,
         [(5, 5, CYAN), (60, 5, CYAN), (5, 40, CYAN), (60, 40, CYAN if fill else WHITE)]),
        ("zero-slices" + suffix, SOLID, "0", "10px", fill,
         [(5, 5, WHITE), (60, 5, WHITE), (5, 40, WHITE), (60, 40, CYAN if fill else WHITE)]),
    ])
CASES.extend([
    ("zero-top-slice", SOLID, "0 25% 25% 25%", "10px", False,
     [(5, 5, WHITE), (60, 5, WHITE), (5, 40, CYAN), (60, 75, CYAN), (60, 40, WHITE)]),
    ("mask-border-overlap", "linear-gradient(black,black)", "80%", "10px", True,
     [(5, 5, CYAN), (115, 75, CYAN), (60, 5, WHITE), (5, 40, WHITE), (60, 40, WHITE)]),
])


def source_css(case) -> str:
    name, source, slices, widths, fill, _ = case
    prefix = "mask-border" if name.startswith("mask-border") else "border-image"
    extra = "background: #1185ab;" if prefix == "mask-border" else ""
    return BASE_CSS + f"div {{ {extra} {prefix}: {source} {slices}{' fill' if fill else ''} / {widths} stretch; }}"


def check(output: Path, *, pdfium: bool = False) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    if pdfium:
        import pypdfium2
    engine = fullbleed.PdfEngine()
    report = dict(schema="fullbleed.border_image_slices.v1", ok=False,
                  version=metadata.version("fullbleed"), cases=[],
                  scope="Interior-color and deterministic-output regressions; not general CSS parity.")
    for case in CASES:
        name, _, _, _, _, probes = case
        folder = output / name
        folder.mkdir(exist_ok=True)
        css = source_css(case)
        (folder / "source.html").write_text(HTML, encoding="utf-8")
        (folder / "source.css").write_text(css, encoding="utf-8")
        path = folder / "output.pdf"
        engine.render_pdf_to_file(HTML, css, str(path))
        pdf = path.read_bytes()
        views = {"native": list(engine.render_image_pages(HTML, css, 96)),
                 "finalized": list(engine.render_finalized_pdf_image_pages(str(path), 96))}
        results = []
        for mode, pages in views.items():
            assert len(pages) == 1, (name, mode, len(pages))
            png = bytes(pages[0])
            (folder / f"{mode}.png").write_bytes(png)
            width, height, rows = decode_png(png)
            assert (width, height) == (160, 120), (name, mode, width, height)
            results.append(dict(mode=mode, probes=[dict(x=x, y=y, expected=rgb,
                actual=list(rows[y + 20][(x + 20) * 4:(x + 20) * 4 + 3])) for x, y, rgb in probes]))
        if pdfium:
            with pypdfium2.PdfDocument(pdf) as doc:
                assert len(doc) == 1, name
                page = doc[0]
                try:
                    bitmap = page.render(scale=96 / 72)
                    image = bitmap.to_pil().convert("RGB")
                    image.save(folder / "pdfium.png")
                    assert image.size == (160, 120), (name, image.size)
                    results.append(dict(mode="pdfium", probes=[dict(x=x, y=y, expected=rgb,
                        actual=list(image.getpixel((x + 20, y + 20)))) for x, y, rgb in probes]))
                    bitmap.close()
                finally:
                    page.close()
        repeated = bytes(engine.render_pdf(HTML, css))
        for result in results:
            result["ok"] = all(max(abs(a - b) for a, b in zip(p["expected"], p["actual"])) <= 3 for p in result["probes"])
        report["cases"].append(dict(name=name, pdf_sha256=hashlib.sha256(pdf).hexdigest(),
                                    deterministic=pdf == repeated, views=results,
                                    ok=pdf == repeated and all(r["ok"] for r in results)))
    report["ok"] = all(case["ok"] for case in report["cases"])
    (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--pdfium", action="store_true")
    args = parser.parse_args()
    report = check(args.out.resolve(), pdfium=args.pdfium)
    print(json.dumps(dict(ok=report["ok"], version=report["version"], cases=len(report["cases"]),
                          failed=[case["name"] for case in report["cases"] if not case["ok"]])))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
