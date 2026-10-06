#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check default Standard 14 preview coverage, overrides and host independence.

Development dependencies: Pillow. The --mask-system-fonts mode must be invoked
inside an isolated Linux mount namespace (sudo unshare --mount --propagation
private ...); it never modifies fonts on the host.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
import math
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile

import fullbleed
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = runpy.run_path(str(ROOT / "tools/smoke_base14_preview.py"))
SPECIMEN = FIXTURE["specimen"]
NAMES = [case[0] for case in FIXTURE["CASES"]]
STAMP = "2026-10-06T00:00:00Z"
# Independent placement fixtures: the Adobe AFMs supply the expected bounds.
COMPONENT_BOUND_FIXTURES = {"radicalex", "arrowvertex", "arrowhorizex", "parenlefttp", "parenleftex", "parenleftbt"}


def mask_system_fonts():
    assert sys.platform == "linux" and os.geteuid() == 0, "Requires Linux root in a private mount namespace"
    assert os.readlink("/proc/self/ns/mnt") != os.readlink("/proc/1/ns/mnt"), "Refusing to mask the host mount namespace"
    # Ensure bind mounts cannot propagate back to any peer namespace.
    subprocess.run(["mount", "--make-rprivate", "/"], check=True)
    directory = tempfile.TemporaryDirectory(prefix="fullbleed-empty-fonts-")
    masked = []
    for path in [Path("/usr/share/fonts"), Path("/usr/local/share/fonts"), Path.home() / ".fonts", Path.home() / ".local/share/fonts"]:
        if path.is_dir():
            subprocess.run(["mount", "--bind", directory.name, str(path)], check=True)
            assert not any(path.iterdir()), path
            masked.append(str(path))
    os.environ.pop("FULLBLEED_FONT_DIR", None)
    return directory, masked


def pdf_page(name, stream, width, height, encoding=""):
    font = f"<< /Type /Font /Subtype /Type1 /BaseFont /{name} {encoding} >>".encode()
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".encode(),
               font, b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream"]
    data = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(data)
    data.extend(b"xref\n0 6\n0000000000 65535 f \n")
    for offset in offsets:
        data.extend(f"{offset:010d} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(data)


def glyph_grid(name, entries, differences):
    columns, size = 8, 44
    rows = math.ceil(len(entries) / columns)
    width, height = 24 + columns * size, 24 + rows * size
    cells, commands = [], []
    for index, entry in enumerate(entries):
        code = index + 1 if differences else entry["code"]
        x, y = 12 + index % columns * size, height - 12 - (index // columns + 1) * size
        commands.append(f"BT /F1 18 Tf 1 0 0 1 {x + 10} {y + 16} Tm <{code:02x}> Tj ET")
        cells.append(dict(glyph=entry["name"], code=code, afm_bbox=entry["bbox"], bounds=[x, height-y-size, x+size, height-y]))
    encoding = ""
    if differences:
        encoding = "/Encoding << /Differences [1 " + " ".join("/" + entry["name"] for entry in entries) + "] >>"
    return pdf_page(name, "\n".join(commands).encode(), width, height, encoding), cells


def afm_entries(name):
    result = []
    for line in (ROOT / "tools/data/base14" / (name + ".afm")).read_text(encoding="latin-1").splitlines():
        if line.startswith("C "):
            fields = dict(part.strip().split(" ", 1) for part in line.split(";") if part.strip())
            result.append(dict(name=fields["N"], code=int(fields["C"]), bbox=[int(value) for value in fields["B"].split()]))
    return result


def check(out, masked):
    out.mkdir(parents=True, exist_ok=False)
    report = dict(schema="fullbleed.standard_font_portability.v1", ok=False,
                  checked_at=datetime.now(timezone.utc).isoformat(), version=metadata.version("fullbleed"),
                  platform=sys.platform, masked_font_directories=masked, cases=[], pdf_sources_unchanged=True,
                  scope="Default native outline coverage and portability; not original-design parity or PDF conformance.")
    engine = fullbleed.PdfEngine(document_timestamp=STAMP)
    fonts = resources.files("fullbleed_assets").joinpath("fonts")

    def save_png(label, png):
        (out / (label + ".png")).write_bytes(png)
        with Image.open(BytesIO(png)) as im:
            pixels = im.convert("RGB")
        return pixels, dict(png_sha256=sha256(png).hexdigest(), pixels_sha256=sha256(pixels.tobytes()).hexdigest(), size=list(pixels.size))

    def render(active, label, pdf):
        path = out / (label + ".pdf")
        path.write_bytes(pdf)
        pages = list(active.render_finalized_pdf_image_pages(str(path), 72))
        assert len(pages) == 1, label
        image, details = save_png(label, bytes(pages[0]))
        assert path.read_bytes() == pdf, "Preview modified its input PDF"
        details["pdf_sha256"] = sha256(pdf).hexdigest()
        return image, details

    try:
        for name in NAMES:
            entries = afm_entries(name)
            grids = [("builtin", [entry for entry in entries if entry["code"] >= 0], False)]
            supported = [entry for entry in entries if (name, entry["name"]) != ("Symbol", "apple")]
            grids.extend((f"differences-{offset // 200 + 1}", supported[offset:offset+200], True) for offset in range(0, len(supported), 200))
            for mode, glyphs, differences in grids:
                label = name + "-" + mode
                pdf, cells = glyph_grid(name, glyphs, differences)
                image, details = render(engine, label, pdf)
                missing, checked_bounds = [], []
                for cell in cells:
                    crop = image.crop(cell["bounds"])
                    ink_box = ImageChops.difference(crop, Image.new("RGB", crop.size, "white")).getbbox()
                    has_ink = ink_box is not None
                    if has_ink != (cell["glyph"] != "space"):
                        missing.append(cell["glyph"])
                    if name == "Symbol" and cell["glyph"] in COMPONENT_BOUND_FIXTURES:
                        left, bottom, right, top = cell["afm_bbox"]
                        expected = [math.floor(10 + left*.018), math.floor(28 - top*.018),
                                    math.ceil(10 + right*.018), math.ceil(28 - bottom*.018)]
                        assert ink_box and all(abs(a-b) <= 1 for a,b in zip(ink_box, expected)), (label, cell["glyph"], ink_box, expected)
                        checked_bounds.append(dict(glyph=cell["glyph"], actual=list(ink_box), expected=expected))
                record = dict(name=label, mode=mode, glyphs=len(cells), missing=missing, checked_component_bounds=checked_bounds, **details)
                report["cases"].append(record)
                (out / (label + ".cells.json")).write_text(json.dumps(cells, indent=2) + "\n", encoding="utf-8")
                assert not missing, (label, missing)

            default, default_details = render(engine, name + "-default", SPECIMEN(name))
            # An explicitly registered different design must remain authoritative.
            bundle = fullbleed.AssetBundle()
            file = {"Symbol": "NotoSansMath-Regular.ttf", "ZapfDingbats": "NotoSansSymbols2-Regular.ttf"}.get(name, "NotoSans-Regular.ttf")
            bundle.add_file(str(fonts.joinpath(file)), fullbleed.AssetKind.Font, name=name)
            registered = fullbleed.PdfEngine(document_timestamp=STAMP)
            registered.register_bundle(bundle)
            override, override_details = render(registered, name + "-registered", SPECIMEN(name))
            changed = ImageChops.difference(default, override).getbbox() is not None
            # The symbols fixture can legitimately select the same Noto
            # outlines. Every Latin fixture uses a different registered design.
            if name not in ("Symbol", "ZapfDingbats"):
                assert changed, name + ": explicit font ignored"
            report["cases"].append(dict(name=name + "-override", mode="registered", different_from_default=changed, **override_details))
            if name not in ("Symbol", "ZapfDingbats"):
                html = "<p>Invoice AV 105 &amp; caf&#233;</p>"
                css = f'@page {{size:300pt 100pt;margin:12pt}} p {{margin:0;font-family:"{name}";font-size:20pt}}'
                direct = bytes(engine.render_image_pages(html, css, 72)[0])
                image, details = save_png(name + "-html", direct)
                assert ImageChops.difference(image, Image.new("RGB", image.size, "white")).getbbox() is not None, name
                pdf = bytes(engine.render_pdf(html, css))
                (out / (name + "-html.pdf")).write_bytes(pdf)
                details["pdf_sha256"] = sha256(pdf).hexdigest()
                trace = engine.export_render_time_font_resolution_trace(html, css)
                assert trace["fonts"] and all(row["raster_target"]["outcome"] == "bundled_substitute" for row in trace["fonts"]), trace
                assert trace["summary"]["raster_system_fallback_count"] == 0
                report["cases"].append(dict(name=name + "-html", mode="html", **details))

        # Embedded programs must survive replacement of an identically named
        # engine font. Use a fresh reader with no registered font as a control.
        embedded_engine = fullbleed.PdfEngine(document_timestamp=STAMP)
        embedded_bundle = fullbleed.AssetBundle()
        embedded_bundle.add_file(str(fonts.joinpath("NotoSans-Regular.ttf")), fullbleed.AssetKind.Font, name="Helvetica")
        embedded_engine.register_bundle(embedded_bundle)
        html = "<p>Embedded font control: Win 105</p>"
        css = '@page {size:320pt 100pt;margin:16pt} p {font-family:"Helvetica";font-size:20pt;margin:0}'
        pdf = bytes(embedded_engine.render_pdf(html, css))
        assert b"/FontFile2" in pdf, "Control must contain an embedded font program"
        fresh, details = render(engine, "embedded-fresh", pdf)
        original, _ = render(embedded_engine, "embedded-original", pdf)
        assert ImageChops.difference(fresh, original).getbbox() is None
        assert ImageChops.difference(fresh, Image.new("RGB", fresh.size, "white")).getbbox() is not None
        report["cases"].append(dict(name="embedded-control", mode="embedded", same_pixels=True, **details))
        assert fullbleed.build_features()["bundled_standard_font_previews"] is True
        report["ok"] = True
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--mask-system-fonts", action="store_true")
    parser.add_argument("--compare", type=Path, help="Compare case pixel and PDF hashes with another platform's verification.json")
    args = parser.parse_args()
    temporary, masked = mask_system_fonts() if args.mask_system_fonts else (None, [])
    report = check(args.out.resolve(), masked)
    if args.compare:
        reference = json.loads(args.compare.read_text(encoding="utf-8"))
        expected = {row["name"]: (row["pixels_sha256"], row["pdf_sha256"]) for row in reference["cases"]}
        actual = {row["name"]: (row["pixels_sha256"], row["pdf_sha256"]) for row in report["cases"]}
        assert reference["ok"] and actual == expected, "Cross-platform pixel/PDF mismatch"
    print(json.dumps(dict(ok=report["ok"], cases=len(report["cases"]), masked=masked, compared=bool(args.compare), output=str(args.out.resolve()))))
    # Mounts disappear with this process's namespace; TemporaryDirectory owns
    # only the empty source directory, never any installed font directories.
    if temporary:
        temporary.cleanup()


if __name__ == "__main__":
    main()
