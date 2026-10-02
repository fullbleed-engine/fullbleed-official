#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check installed-engine gradient previews against known interior colors."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata
import json
from pathlib import Path
import runpy
import sys

import fullbleed


# Reuse the fixture runner's stdlib PNG reader; no imaging dependency is needed.
decode_png = runpy.run_path(str(Path(__file__).with_name("run_css_fixture_suite.py")))["_decode_png_rgba_rows"]

CASES = [
    ("linear", "linear-gradient(90deg,red,blue)", "", "",
     [(25, 50, [190, 0, 65]), (75, 50, [62, 0, 193])]),
    ("report-colors", "linear-gradient(180deg,#d9bc77,#a9b884)", "", "",
     [(50, 25, [205, 187, 122]), (50, 75, [181, 185, 129])]),
    ("hard-linear", "linear-gradient(90deg,red 0%,red 50%,blue 50%,blue 100%)", "", "",
     [(25, 50, [255, 0, 0]), (75, 50, [0, 0, 255])]),
    ("radial", "radial-gradient(circle 40pt at 50pt 50pt,red,blue)", "", "",
     [(60, 50, [188, 0, 67]), (90, 50, [0, 0, 255])]),
    ("hard-radial", "radial-gradient(circle 40pt at 50pt 50pt,red 0%,red 50%,blue 50%,blue 100%)", "", "",
     [(60, 50, [255, 0, 0]), (80, 50, [0, 0, 255])]),
    ("elliptical", "radial-gradient(ellipse 40pt 20pt at 50pt 50pt,red,blue)", "", "",
     [(70, 50, [124, 0, 131]), (50, 90, [0, 0, 255])]),
    ("translucent", "linear-gradient(90deg,rgba(255,0,0,.5),rgba(0,0,255,.5))", "", "background:#00ff00;",
     [(25, 50, [95, 128, 33]), (75, 50, [31, 128, 97])]),
    ("varying-alpha", "linear-gradient(90deg,rgba(255,0,0,0),rgba(0,0,255,1))", "", "background:#00ff00;",
     [(25, 50, [0, 190, 65]), (75, 50, [0, 62, 193])]),
    ("radial-alpha", "radial-gradient(circle 40pt at 50pt 50pt,rgba(255,0,0,.5),rgba(0,0,255,.5))", "", "background:#00ff00;",
     [(60, 50, [94, 128, 34]), (90, 50, [0, 128, 128])]),
    ("rounded-clip", "linear-gradient(90deg,red,blue)", "border-radius:24pt;", "",
     [(1, 1, [255, 255, 255]), (75, 50, [62, 0, 193])]),
]


def check(output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    engine = fullbleed.PdfEngine()
    report = dict(schema="fullbleed.gradient_preview_smoke.v1", ok=False,
                  checked_at=datetime.now(timezone.utc).isoformat(),
                  fullbleed_version=metadata.version("fullbleed"), python=sys.version,
                  scope="Interior-color regression checks on finalized PDF previews; not general PDF renderer parity or conformance.",
                  cases=[])
    try:
        for name, background, style, body, probes in CASES:
            folder = output / name
            folder.mkdir(exist_ok=True)
            html = "<!doctype html><html><body><div></div></body></html>"
            css = ("@page {size:100pt 100pt;margin:0} body {margin:0;" + body + "} "
                   "div {width:100pt;height:100pt;background:" + background + ";" + style + "}")
            (folder / "source.html").write_text(html, encoding="utf-8")
            (folder / "source.css").write_text(css, encoding="utf-8")
            source = folder / "output.pdf"
            engine.render_pdf_to_file(html, css, str(source))
            pdf = source.read_bytes()
            for dpi in [72, 144]:
                pages = list(engine.render_finalized_pdf_image_pages(str(source), dpi))
                assert len(pages) == 1, name
                png = bytes(pages[0])
                (folder / f"preview-{dpi}.png").write_bytes(png)
                width, height, rows = decode_png(png)
                assert (width, height) == (100 * dpi // 72, 100 * dpi // 72)
                checks = []
                for x, y, expected in probes:
                    pixel_x, pixel_y = x * dpi // 72, y * dpi // 72
                    actual = list(rows[pixel_y][pixel_x * 4:pixel_x * 4 + 3])
                    error = max(abs(a - b) for a, b in zip(actual, expected))
                    checks.append(dict(point_pt=[x, y], expected=expected, actual=actual, max_channel_error=error))
                repeat = bytes(list(engine.render_finalized_pdf_image_pages(str(source), dpi))[0])
                result = dict(name=name, dpi=dpi, width=width, height=height, probes=checks,
                              deterministic=png == repeat, source_unchanged=pdf == source.read_bytes(),
                              pdf_sha256=hashlib.sha256(pdf).hexdigest(), png_sha256=hashlib.sha256(png).hexdigest())
                report["cases"].append(result)
                assert result["deterministic"] and result["source_unchanged"], name
                assert all(item["max_channel_error"] <= 4 for item in checks), f"{name} at {dpi} DPI: {checks}"
        report["ok"] = True
    finally:
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    report = check(args.out.resolve())
    print(json.dumps(dict(ok=report["ok"], version=report["fullbleed_version"],
                          cases=len(report["cases"]), output=str(args.out.resolve()))))


if __name__ == "__main__":
    main()
