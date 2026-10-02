#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare native Standard 14 previews with explicit PDF reference widths."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata, resources
import json
from pathlib import Path
import sys

import fullbleed


# Widths for space, W, i from the pinned Adobe AFMs in tools/data/base14.
# Keep these small reference fixtures independent of the runtime table generator.
CASES = [
    ("Courier", 600, 600, 600),
    ("Courier-Bold", 600, 600, 600),
    ("Courier-Oblique", 600, 600, 600),
    ("Courier-BoldOblique", 600, 600, 600),
    ("Helvetica", 278, 944, 222),
    ("Helvetica-Bold", 278, 944, 278),
    ("Helvetica-Oblique", 278, 944, 222),
    ("Helvetica-BoldOblique", 278, 944, 278),
    ("Times-Roman", 250, 944, 278),
    ("Times-Bold", 250, 1000, 278),
    ("Times-Italic", 250, 833, 278),
    ("Times-BoldItalic", 250, 889, 278),
    ("Symbol", 250, 768, 329),
    ("ZapfDingbats", 278, 776, 713),
]


def specimen(name, *, widths=None, encoding="", text=b"iWi Wi", spacing=False):
    """Make a minimal test PDF with an ordinary Type 1 font and valid xref."""
    font = f"<< /Type /Font /Subtype /Type1 /BaseFont /{name} {encoding}"
    if widths is not None:
        first, last = min(widths), max(widths)
        entries = " ".join(str(widths.get(code, 500)) for code in range(first, last + 1))
        font += f" /FirstChar {first} /LastChar {last} /Widths [{entries}]"
    font += " >>"
    controls = b"2 Tc 3 Tw 75 Tz " if spacing else b""
    stream = b"BT /F1 24 Tf 18 62 Td " + controls + b"<" + text.hex().encode() + b"> Tj ET"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 100] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        font.encode("ascii"),
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream",
    ]
    result = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(result)
    result.extend(b"xref\n0 6\n0000000000 65535 f \n")
    for offset in offsets:
        result.extend(f"{offset:010d} 00000 n \n".encode())
    result.extend(f"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(result)


def check(output: Path, only: str | None = None) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    fonts = resources.files("fullbleed_assets").joinpath("fonts")
    # Bind known Unicode outlines explicitly so this advance test neither depends
    # on system fonts nor silently passes when an unavailable substitute is blank.
    bundle = fullbleed.AssetBundle()
    for name, *_ in CASES:
        filename = {"Symbol": "NotoSansMath-Regular.ttf",
                    "ZapfDingbats": "NotoSansSymbols2-Regular.ttf"}.get(name, "NotoSans-Regular.ttf")
        bundle.add_file(str(fonts.joinpath(filename)), fullbleed.AssetKind.Font, name=name)
    engine = fullbleed.PdfEngine()
    engine.register_bundle(bundle)
    report = dict(schema="fullbleed.base14_preview_smoke.v1", ok=False,
                  checked_at=datetime.now(timezone.utc).isoformat(),
                  fullbleed_version=metadata.version("fullbleed"), python=sys.version,
                  cases=[], outline_source="Bundled Noto faces explicitly registered under fixture font names",
                  scope="Same-runtime preview comparison with explicit Adobe reference widths; not outline parity or PDF conformance.")

    def render(folder, label, pdf):
        source = folder / (label + ".pdf")
        source.write_bytes(pdf)
        pages = list(engine.render_finalized_pdf_image_pages(str(source), 96))
        assert len(pages) == 1, label
        png = bytes(pages[0])
        assert png.startswith(b"\x89PNG\r\n\x1a\n"), label
        (folder / (label + ".png")).write_bytes(png)
        assert source.read_bytes() == pdf, "preview changed its source PDF"
        return png

    variants = []
    for name, space, wide, narrow in CASES:
        if only and only != name:
            continue
        for spacing in [False, True]:
            variants.append((name + ("-spacing" if spacing else ""), name,
                             {32: space, 87: wide, 105: narrow}, "", b"iWi Wi", spacing))
    if not only:
        variants += [
            ("Helvetica-WinAnsi", "Helvetica", {39: 191, 87: 944, 105: 222, 128: 556},
             "/Encoding /WinAnsiEncoding", b"\x80i'Wi", False),
            ("Helvetica-Differences", "Helvetica", {65: 944, 66: 222, 67: 500},
             "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /W /i /fi] >>",
             b"ABCBAB", True),
        ]
    assert variants, only
    try:
        for label, name, widths, encoding, text, spacing in variants:
            folder = output / label
            folder.mkdir(exist_ok=True)
            options = dict(encoding=encoding, text=text, spacing=spacing)
            actual = render(folder, "implicit", specimen(name, **options))
            expected = render(folder, "explicit", specimen(name, widths=widths, **options))
            blank = render(folder, "blank", specimen(name, text=b""))
            incorrect = render(folder, "incorrect", specimen(name, widths={code: 500 for code in widths}, **options))
            result = dict(name=label, matches_reference=actual == expected,
                          nonblank=actual != blank, rejects_incorrect_widths=expected != incorrect,
                          png_sha256=hashlib.sha256(actual).hexdigest())
            report["cases"].append(result)
            assert result["matches_reference"], f"{label}: omitted widths disagree with reference"
            assert result["nonblank"], f"{label}: preview is blank"
            assert result["rejects_incorrect_widths"], f"{label}: negative control was not detected"
        report["ok"] = True
    finally:
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--font", choices=[case[0] for case in CASES])
    args = parser.parse_args()
    report = check(args.out.resolve(), args.font)
    print(json.dumps(dict(ok=report["ok"], fullbleed_version=report["fullbleed_version"],
                          cases=len(report["cases"]), output=str(args.out.resolve()))))


if __name__ == "__main__":
    main()
