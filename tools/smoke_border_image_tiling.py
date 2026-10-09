#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check complete space tiles and center scaling in installed rendered output.

Expected colors follow CSS Backgrounds 3 section 5.6, independently checked in
Chrome's printed PDFs. A space region with no complete tile remains empty.
Center scaling skips a zero edge scale, then uses the opposite edge or the
unscaled source. Repeat centers its first tile, including even tile counts.
https://www.w3.org/TR/css-backgrounds-3/#border-image-process

Native preview checks require only the installed package and Python's standard
library. --pdfium additionally inspects both direct and compiled PDFs with the
optional pypdfium2 test dependency.
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
COLORS = {"white": [255, 255, 255], "red": [255, 0, 0], "blue": [0, 0, 255],
          "orange": [255, 128, 0], "green": [0, 180, 0], "purple": [128, 0, 180]}
HTML = '<!doctype html><html><body><div class="box"></div></body></html>'


def raster_source(vertical: bool) -> str:
    """An explicit nine-slice image; only its center contains a stripe pattern."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = bytearray()
    for y in range(30):
        rows.append(0)
        for x in range(30):
            col, row = x // 10, y // 10
            if col == 1 and row == 1:
                name = "green" if (y % 10 if vertical else x % 10) < 5 else "purple"
            elif col == 1:
                name = "blue"
            elif row == 1:
                name = "orange"
            else:
                name = "red"
            rows.extend(COLORS[name])
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 30, 30, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
    return 'url("data:image/png;base64,' + base64.b64encode(png).decode("ascii") + '")'


def svg_source(vertical: bool) -> str:
    rects = []
    def rect(x, y, width, height, color):
        rgb = COLORS[color]
        fill = "#" + "".join(f"{value:02x}" for value in rgb)
        rects.append(f'<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="{fill}"/>')
    for row in range(3):
        for col in range(3):
            if row == 1 and col == 1:
                continue
            rect(col * 10, row * 10, 10, 10, "blue" if col == 1 else "orange" if row == 1 else "red")
    if vertical:
        rect(10, 10, 10, 5, "green")
        rect(10, 15, 10, 5, "purple")
    else:
        rect(10, 10, 5, 10, "green")
        rect(15, 10, 5, 10, "purple")
    svg = '<svg xmlns="http://www.w3.org/2000/svg" width="30" height="30">' + "".join(rects) + "</svg>"
    return 'url("data:image/svg+xml,' + urllib.parse.quote(svg, safe="") + '")'


# Each probe is inside a region, away from interpolation and clipped tile edges.
CASES = [
    ("space-no-horizontal-tile", 26, 60, "10px", "space stretch", False,
     [(33, 25, "white"), (33, 40, "white"), (25, 25, "red"), (25, 40, "orange")]),
    ("space-no-vertical-tile", 60, 26, "10px", "stretch space", True,
     [(25, 33, "white"), (40, 33, "white"), (25, 25, "red"), (40, 25, "blue")]),
    ("space-no-tile-both-axes", 26, 26, "10px", "space", False,
     [(33, 25, "white"), (25, 33, "white"), (33, 33, "white"), (25, 25, "red")]),
    ("space-one-complete-tile", 39, 60, "10px", "space stretch", False,
     [(32, 25, "white"), (38, 25, "blue"), (47, 25, "white"), (32, 40, "white"), (36, 40, "green")]),
    ("center-zero-top-falls-back-bottom", 60, 70, "0 10px 20px 10px", "repeat stretch", False,
     [(35, 35, "purple"), (45, 35, "green"), (55, 35, "purple"), (65, 35, "green")]),
    ("center-zero-horizontal-edges-unscaled", 60, 60, "0 10px", "repeat stretch", False,
     [(32, 35, "purple"), (37, 35, "green"), (42, 35, "purple")]),
    ("center-zero-left-falls-back-right", 70, 60, "10px 20px 10px 0", "stretch repeat", True,
     [(35, 35, "purple"), (35, 45, "green"), (35, 55, "purple"), (35, 65, "green")]),
    ("center-zero-vertical-edges-unscaled", 60, 60, "10px 0", "stretch repeat", True,
     [(35, 32, "purple"), (35, 37, "green"), (35, 42, "purple")]),
    ("stretch-control", 60, 60, "10px", "stretch", False,
     [(35, 40, "green"), (65, 40, "purple"), (35, 25, "blue")]),
]


def source_css(case, source_kind: str = "raster") -> str:
    _, width, height, border_width, repeat, vertical, _ = case
    source = raster_source(vertical) if source_kind == "raster" else svg_source(vertical)
    return f'''@page {{ size: 140px 130px; margin: 0; }}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
html, body {{ background: white; }}
.box {{ position: absolute; left: 20px; top: 20px; width: {width}px; height: {height}px;
  background: white; border: 10px solid red; border-image-source: {source};
  border-image-slice: 10 fill; border-image-width: {border_width}; border-image-repeat: {repeat}; }}
'''


def check(output: Path, *, pdfium: bool = False) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    if pdfium:
        import pypdfium2
    engine = fullbleed.PdfEngine()
    report = dict(schema="fullbleed.border_image_tiling.v1", version=metadata.version("fullbleed"),
                  ok=False, cases=[], scope="Interior colors and deterministic output; not general CSS parity.")
    for source_kind, case in [(kind, case) for kind in ("raster", "svg") for case in CASES]:
        case_name, *_, probes = case
        name = case_name + "-" + source_kind
        css = source_css(case, source_kind)
        folder = output / name
        folder.mkdir(exist_ok=True)
        (folder / "source.html").write_text(HTML, encoding="utf-8")
        (folder / "source.css").write_text(css, encoding="utf-8")
        direct = bytes(engine.render_pdf(HTML, css))
        compiled = engine.compile_pdf(HTML, css)
        compiled_pdf = bytes(compiled.render_pdf())
        pdfs = {"direct": direct, "compiled": compiled_pdf}
        for label, data in pdfs.items():
            (folder / (label + ".pdf")).write_bytes(data)
        views = []
        for label, pages in [
            ("native", list(engine.render_image_pages(HTML, css, 96))),
            ("finalized", list(engine.render_finalized_pdf_image_pages(str(folder / "direct.pdf"), 96))),
        ]:
            assert len(pages) == 1, (name, label, len(pages))
            png = bytes(pages[0])
            (folder / (label + ".png")).write_bytes(png)
            width, height, rows = decode_png(png)
            assert (width, height) == (140, 130), (name, label, width, height)
            views.append(dict(mode=label, probes=[dict(x=x, y=y, expected=COLORS[color],
                actual=list(rows[y][x * 4:x * 4 + 3])) for x, y, color in probes]))
        if pdfium:
            for label, data in pdfs.items():
                with pypdfium2.PdfDocument(data) as doc:
                    assert len(doc) == 1, (name, label)
                    page = doc[0]
                    try:
                        bitmap = page.render(scale=96 / 72)
                        image = bitmap.to_pil().convert("RGB")
                        image.save(folder / (label + "-pdfium.png"))
                        assert image.size == (140, 130), (name, label, image.size)
                        views.append(dict(mode=label + "-pdfium", probes=[dict(x=x, y=y, expected=COLORS[color],
                            actual=list(image.getpixel((x, y)))) for x, y, color in probes]))
                        bitmap.close()
                    finally:
                        page.close()
        for view in views:
            view["ok"] = all(max(abs(a-b) for a, b in zip(p["expected"], p["actual"])) <= 3 for p in view["probes"])
        deterministic = direct == bytes(engine.render_pdf(HTML, css)) and compiled_pdf == bytes(compiled.render_pdf())
        report["cases"].append(dict(name=name, pdfs={k: hashlib.sha256(v).hexdigest() for k,v in pdfs.items()},
                                    deterministic=deterministic, views=views,
                                    ok=deterministic and all(view["ok"] for view in views)))
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
