#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify one searchable text copy in installed-engine styled PDFs.

pypdf is a test-only reader, never a Fullbleed runtime dependency.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata, resources
import json
from pathlib import Path
import runpy
import sys

import fullbleed
from pypdf import PdfReader


decode_png = runpy.run_path(str(Path(__file__).with_name("run_css_fixture_suite.py")))["_decode_png_rgba_rows"]


def ink(png: bytes) -> int:
    width, _, rows = decode_png(png)
    return sum(width * 3 * 255 - sum(row[0::4]) - sum(row[1::4]) - sum(row[2::4])
               for row in rows)


CASES = [
    ("quickstart", "<h1>Invoice INV-1042</h1><p>Consulting: USD 1,200.00</p>",
     "h1 { color: #175c52 }", "Invoice INV-1042 Consulting: USD 1,200.00"),
    ("repeated", "<p>ABBA ABBA 2026</p>", "p {font-weight:700}", "ABBA ABBA 2026"),
    ("small-heading", "<h1>Small heading</h1>", "h1 {font-size:9pt}", "Small heading"),
    ("large-bold", "<p>Large bold 48</p>", "p {font-size:48pt;font-weight:700}", "Large bold 48"),
    ("bold-italic", "<p>Bold italic office</p>", "p {font-weight:700;font-style:italic}", "Bold italic office"),
    ("unicode", "<p>Résumé café office</p>", "p {font-weight:700}", "Résumé café office"),
    ("mixed", "<p>Plain <strong>bold</strong> plain <em>italic</em>.</p>", "", "Plain bold plain italic."),
    ("opacity", "<p>Translucent heading</p>", "p {font-weight:700;opacity:.65;color:#175c52}", "Translucent heading"),
    ("transform", "<p>Rotated heading</p>", "p {font-weight:700;transform:rotate(4deg);transform-origin:left top}", "Rotated heading"),
    ("scaled", "<p>Scaled heading</p>", "p {font-weight:700;transform:scale(1.2,.8);transform-origin:left top}", "Scaled heading"),
    ("sheared", "<p>Sheared heading</p>", "p {font-weight:700;transform:skewX(12deg);transform-origin:left top}", "Sheared heading"),
    ("shadow", "<p>Shadow heading</p>", "p {font-weight:700;text-shadow:1pt 1pt 2pt #888}", "Shadow heading"),
]


def check(output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    report = dict(schema="fullbleed.text_extraction_smoke.v1", ok=False,
                  checked_at=datetime.now(timezone.utc).isoformat(),
                  fullbleed_version=metadata.version("fullbleed"),
                  reader_version=metadata.version("pypdf"), python=sys.version,
                  scope="Single-copy text extraction and native preview smoke; not PDF profile conformance.",
                  cases=[])
    try:
        for profile in ["none", "tagged", "pdfua-1"]:
            engine = fullbleed.PdfEngine(pdf_profile=profile, document_lang="en-US",
                                        document_title="Text extraction regression")
            bundle = fullbleed.AssetBundle()
            bundle.add_file(str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")),
                            fullbleed.AssetKind.Font, name="Inter")
            engine.register_bundle(bundle)
            for name, html, style, expected in CASES:
                folder = output / profile / name
                folder.mkdir(parents=True, exist_ok=True)
                css = "@page {size:A4;margin:20mm} body {font-family:Inter} " + style
                (folder / "source.html").write_text(html, encoding="utf-8")
                (folder / "source.css").write_text(css, encoding="utf-8")
                source = folder / "output.pdf"
                engine.render_pdf_to_file(html, css, str(source))
                pdf = source.read_bytes()
                reader = PdfReader(source)
                text = " ".join(" ".join(page.extract_text() for page in reader.pages).split())
                previews = list(engine.render_finalized_pdf_image_pages(str(source), 96))
                for index, png in enumerate(previews, 1):
                    (folder / f"page-{index}.png").write_bytes(bytes(png))
                direct = list(engine.render_image_pages(html, css, 96))
                assert len(direct) == len(previews) == 1, f"{profile}/{name}"
                reference_png = bytes(direct[0])
                (folder / "direct-preview.png").write_bytes(reference_png)
                reference_ink = ink(reference_png)
                preview_ink = ink(bytes(previews[0]))
                # A broad coverage bound catches blank glyphs or solid boxes
                # while allowing font/path rasterization differences. This is
                # a visual regression control, not a pixel-parity assertion.
                coverage_ratio = preview_ink / reference_ink if reference_ink else 0
                result = dict(profile=profile, name=name, expected=expected, extracted=text,
                              pages=len(reader.pages), previews=len(previews),
                              pdf_sha256=hashlib.sha256(pdf).hexdigest(),
                              preview_sha256=[hashlib.sha256(bytes(png)).hexdigest() for png in previews],
                              source_unchanged=source.read_bytes() == pdf,
                              preview_ink_ratio=coverage_ratio)
                result["ok"] = (text == expected and result["pages"] == result["previews"] == 1
                                and result["source_unchanged"] and .8 <= coverage_ratio <= 1.2)
                report["cases"].append(result)
        report["ok"] = all(case["ok"] for case in report["cases"])
        failed = [f"{case['profile']}/{case['name']}: {case['extracted']!r}, ink ratio={case['preview_ink_ratio']:.3f}"
                  for case in report["cases"] if not case["ok"]]
        assert report["ok"], "Single-copy text regression: " + "; ".join(failed)
    finally:
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out.resolve())
    print(json.dumps(dict(ok=result["ok"], cases=len(result["cases"]), output=str(args.out.resolve()))))


if __name__ == "__main__":
    main()
