#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify signed spread, translated holes and blur support in inset shadows.

Interior color probes follow CSS Backgrounds 3 section 6.1.1 and have also been
checked against printed browser output. The blurred straight edge is compared
with an independent Gaussian CDF, using the authored blur radius / 2 as sigma.
Whole-region comparisons at 300 DPI reject stray paint on unshadowed edges.
https://www.w3.org/TR/css-backgrounds-3/#shadow-shape
https://www.w3.org/TR/css-backgrounds-3/#shadow-blur

Native and finalized previews use only Fullbleed and the standard library.
--pdfium also checks actual direct and compiled PDFs using the optional
pypdfium2 development dependency. No browser is needed to run these checks.
"""
from __future__ import annotations

import argparse
import hashlib
from importlib import metadata
import json
import math
from pathlib import Path
import runpy

import fullbleed


decode_png = runpy.run_path(str(Path(__file__).with_name("run_css_fixture_suite.py")))["_decode_png_rgba_rows"]
COLORS = {"background": (144, 202, 249), "shadow": (1, 87, 155), "white": (255, 255, 255)}
# Every probe is clear of antialiased curves and sharp boundaries. Coordinates
# are on a 160 by 120 CSS-pixel page; the box occupies (20, 20) to (140, 100).
CASES = [
    ("negative-hidden", "8px 0 0 -12px", "0", [(25, 60, "background"), (135, 60, "background")]),
    ("negative-hidden-rounded", "8px 0 0 -12px", "20px", [(25, 60, "background"), (135, 60, "background"), (21, 21, "white")]),
    ("negative-left", "24px 0 0 -12px", "0", [(25, 60, "shadow"), (35, 60, "background"), (135, 60, "background")]),
    ("negative-right", "-24px 0 0 -12px", "0", [(25, 60, "background"), (125, 60, "background"), (135, 60, "shadow")]),
    ("positive-offset", "12px 0 0 8px", "0", [(25, 60, "shadow"), (45, 60, "background"), (135, 60, "background"), (80, 25, "shadow")]),
    ("diagonal-offset", "24px 24px 0 0", "0", [(25, 60, "shadow"), (80, 25, "shadow"), (60, 60, "background"), (135, 90, "background")]),
    ("large-left", "100px 0 0 0", "0", [(25, 60, "shadow"), (105, 60, "shadow"), (125, 60, "background")]),
    ("large-right", "-100px 0 0 0", "0", [(25, 60, "background"), (45, 60, "shadow"), (135, 60, "shadow")]),
    ("large-down", "0 60px 0 0", "0", [(80, 25, "shadow"), (80, 65, "shadow"), (80, 90, "background")]),
    ("large-up", "0 -60px 0 0", "0", [(80, 25, "background"), (80, 45, "shadow"), (80, 90, "shadow")]),
    ("rounded-offset", "20px 8px 0 0", "20px", [(45, 30, "shadow"), (40, 40, "shadow"), (65, 60, "background"), (21, 21, "white")]),
    ("elliptical-negative", "24px 12px 0 -12px", "6px 14px 12px 20px / 12px 20px 6px 14px", [(25, 60, "shadow"), (45, 25, "background"), (80, 60, "background")]),
    ("collapsed-hole", "20px 0 0 50px", "20px", [(25, 60, "shadow"), (80, 60, "shadow"), (135, 60, "shadow"), (21, 21, "white")]),
    ("translated-away", "160px 0 0 0", "20px", [(25, 60, "shadow"), (80, 60, "shadow"), (135, 60, "shadow"), (21, 21, "white")]),
    ("blur-translated-away", "160px 0 12px 0", "20px", [(25, 60, "shadow"), (80, 60, "shadow"), (135, 60, "shadow"), (80, 25, "shadow"), (21, 21, "white")]),
    ("blur-negative", "24px 0 12px -12px", "0", [(80, 60, "background"), (135, 60, "background")]),
    ("empty-control", "0 0 0 0", "20px", [(25, 60, "background"), (80, 25, "background"), (80, 60, "background"), (21, 21, "white")]),
]


def fixture(case, element="block"):
    name, shadow, radius, _ = case
    content = '<table><tbody><tr><td class="box"></td></tr></tbody></table>' if element == "cell" else '<div class="box"></div>'
    inset = 0 if element == "page-start" else 40 if element == "page-end" else 20
    html = f'<!doctype html><html><body><div class="stage">{content}</div></body></html>'
    css = f'''@page {{ size: 160px 120px; margin: 0; }}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
html, body {{ background: white; }}
.stage {{ padding: {inset}px; width: 160px; height: 120px; }}
table {{ border-collapse: separate; border-spacing: 0; }}
.box {{ width: 120px; height: 80px; background: #90caf9; border-radius: {radius};
  box-shadow: inset {shadow} #01579b; }}
'''
    if element in ("page-start", "page-end"):
        css += f'.box {{ position: absolute; left: {inset}px; top: {inset}px; }}\n'
    return html, css


def probes_for(case, element="block"):
    probes = [dict(x=x, y=y, expected=COLORS[color], tolerance=3) for x, y, color in case[3]]
    probes.extend(dict(x=x, y=y, expected=COLORS["white"], tolerance=3)
                  for x, y in [(10, 60), (150, 60), (80, 10), (80, 110)])
    if case[0] == "blur-negative":
        # The unblurred left edge is x=32, with sigma=6. Other hole boundaries
        # are over six sigma away from these center-line probes.
        for x in (23, 29, 35, 41, 47):
            coverage = 0.5 * (1 + math.erf((32 - (x + 0.5)) / (6 * math.sqrt(2))))
            expected = [round(bg * (1-coverage) + fg * coverage)
                        for bg, fg in zip(COLORS["background"], COLORS["shadow"])]
            probes.append(dict(x=x, y=60, expected=expected, tolerance=10))
    shift = -20 if element == "page-start" else 20 if element == "page-end" else 0
    return [{**p, "x": p["x"] + shift, "y": p["y"] + shift} for p in probes
            if 0 <= p["x"] + shift < 160 and 0 <= p["y"] + shift < 120]


def selected_cases():
    selected = [(case, "block") for case in CASES]
    selected += [(case, "cell") for case in CASES if case[0] in
                 ("negative-hidden", "negative-left", "positive-offset", "large-left", "large-down", "blur-negative")]
    selected += [(case, edge) for edge in ("page-start", "page-end") for case in CASES
                 if case[0] in ("blur-translated-away", "blur-negative")]
    return selected


def check_clear_region(engine, html, css, case, folder, pdfs, *, pdfium=False):
    """Check the complete unshadowed region, including fractional raster edges."""
    if case[0] not in ("negative-hidden", "negative-hidden-rounded", "diagonal-offset"):
        return []
    reference_css = css.replace(f"box-shadow: inset {case[1]} #01579b;", "box-shadow: none;")
    assert reference_css != css
    reference_pdf = bytes(engine.render_pdf(html, reference_css))
    reference_path = folder / "clear-reference.pdf"
    reference_path.write_bytes(reference_pdf)
    start = math.ceil(46 * 300 / 96) if case[0] == "diagonal-offset" else 0
    checks = []

    def compare(mode, actual, reference):
        (folder / f"{mode}-300.png").write_bytes(actual)
        (folder / f"{mode}-300-reference.png").write_bytes(reference)
        width, height, rows = decode_png(actual)
        ref_width, ref_height, ref_rows = decode_png(reference)
        assert (width, height) == (ref_width, ref_height)
        # PDF readers may ceil a fractional device size one pixel upward.
        assert width in (500, 501) and height in (375, 376)
        different = sum(a[i:i+4] != b[i:i+4] for a, b in zip(rows[start:], ref_rows[start:])
                        for i in range(start*4, width*4, 4))
        checks.append(dict(mode=mode, dpi=300, region=[start, start, width, height],
                           differing_pixels=different, ok=different == 0))

    for mode, actual, reference in [
        ("native", engine.render_image_pages(html, css, 300), engine.render_image_pages(html, reference_css, 300)),
        ("finalized", engine.render_finalized_pdf_image_pages(str(folder / "direct.pdf"), 300),
         engine.render_finalized_pdf_image_pages(str(reference_path), 300)),
    ]:
        assert len(actual) == len(reference) == 1
        compare(mode, bytes(actual[0]), bytes(reference[0]))
    if pdfium:
        from io import BytesIO
        import pypdfium2

        def raster(data):
            with pypdfium2.PdfDocument(data) as doc:
                assert len(doc) == 1
                page = doc[0]
                bitmap = page.render(scale=300/72)
                try:
                    buffer = BytesIO()
                    bitmap.to_pil().convert("RGBA").save(buffer, format="PNG")
                    return buffer.getvalue()
                finally:
                    bitmap.close()
                    page.close()

        reference = raster(reference_pdf)
        for mode, data in pdfs.items():
            compare(mode+"-pdfium", raster(data), reference)
    return checks


def check(output: Path, *, pdfium=False):
    if pdfium:
        import pypdfium2
    output.mkdir(parents=True, exist_ok=True)
    engine = fullbleed.PdfEngine()
    report = dict(schema="fullbleed.inset_shadows.v1", version=metadata.version("fullbleed"), ok=False,
                  scope="Interior shadow geometry, straight-edge blur, clear regions at 300 DPI, clipping and deterministic output; not complete CSS parity.", cases=[])
    for case, element in selected_cases():
        name = case[0] + "-" + element
        folder = output / name
        folder.mkdir(exist_ok=True)
        html, css = fixture(case, element)
        (folder / "source.html").write_text(html, encoding="utf-8")
        (folder / "source.css").write_text(css, encoding="utf-8")
        direct = bytes(engine.render_pdf(html, css))
        compiled = engine.compile_pdf(html, css)
        pdfs = {"direct": direct, "compiled": bytes(compiled.render_pdf())}
        for mode, data in pdfs.items():
            (folder / f"{mode}.pdf").write_bytes(data)
        views = []
        expected = probes_for(case, element)
        for mode, pages in [
            ("native", list(engine.render_image_pages(html, css, 96))),
            ("finalized", list(engine.render_finalized_pdf_image_pages(str(folder / "direct.pdf"), 96))),
        ]:
            assert len(pages) == 1, (name, mode, len(pages))
            png = bytes(pages[0])
            (folder / f"{mode}.png").write_bytes(png)
            width, height, rows = decode_png(png)
            assert (width, height) == (160, 120), (name, mode, width, height)
            views.append(dict(mode=mode, probes=[{**p, "actual": list(rows[p["y"]][p["x"]*4:p["x"]*4+3])} for p in expected]))
        if pdfium:
            for mode, data in pdfs.items():
                with pypdfium2.PdfDocument(data) as doc:
                    assert len(doc) == 1, (name, mode)
                    page = doc[0]
                    try:
                        bitmap = page.render(scale=96/72)
                        try:
                            image = bitmap.to_pil().convert("RGB")
                            image.save(folder / f"{mode}-pdfium.png")
                            assert image.size == (160, 120), (name, mode, image.size)
                            views.append(dict(mode=mode+"-pdfium", pixel_sha256=hashlib.sha256(image.tobytes()).hexdigest(),
                                probes=[{**p, "actual": list(image.getpixel((p["x"],p["y"])))} for p in expected]))
                        finally:
                            bitmap.close()
                    finally:
                        page.close()
        for view in views:
            view["ok"] = all(max(abs(a-b) for a,b in zip(p["expected"],p["actual"])) <= p["tolerance"] for p in view["probes"])
        equivalent_to_no_shadow = None
        if case[0] == "empty-control":
            reference_css = css.replace("box-shadow: inset 0 0 0 0 #01579b;", "box-shadow: none;")
            assert reference_css != css
            reference_pdf = bytes(engine.render_pdf(html, reference_css))
            reference_path = folder / "no-shadow.pdf"
            reference_path.write_bytes(reference_pdf)
            equivalent_to_no_shadow = (
                bytes(engine.render_image_pages(html, reference_css, 96)[0]) == (folder / "native.png").read_bytes()
                and bytes(engine.render_finalized_pdf_image_pages(str(reference_path), 96)[0]) == (folder / "finalized.png").read_bytes()
            )
            if pdfium:
                with pypdfium2.PdfDocument(reference_pdf) as doc:
                    page = doc[0]
                    bitmap = page.render(scale=96/72)
                    try:
                        reference_digest = hashlib.sha256(bitmap.to_pil().convert("RGB").tobytes()).hexdigest()
                        equivalent_to_no_shadow &= all(v["pixel_sha256"] == reference_digest for v in views if "pixel_sha256" in v)
                    finally:
                        bitmap.close()
                        page.close()
        clear_regions = check_clear_region(engine, html, css, case, folder, pdfs, pdfium=pdfium)
        deterministic = direct == bytes(engine.render_pdf(html, css)) and pdfs["compiled"] == bytes(compiled.render_pdf())
        report["cases"].append(dict(name=name, deterministic=deterministic, views=views, equivalent_to_no_shadow=equivalent_to_no_shadow,
            clear_region_checks=clear_regions,
            pdfs={k:hashlib.sha256(v).hexdigest() for k,v in pdfs.items()},
            ok=deterministic and all(view["ok"] for view in views) and equivalent_to_no_shadow is not False and all(c["ok"] for c in clear_regions)))
    report["ok"] = all(case["ok"] for case in report["cases"])
    (output / "verification.json").write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
    return report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",type=Path,required=True)
    parser.add_argument("--pdfium",action="store_true")
    args=parser.parse_args()
    report=check(args.out.resolve(),pdfium=args.pdfium)
    print(json.dumps(dict(ok=report["ok"],version=report["version"],cases=len(report["cases"]),
        failed=[case["name"] for case in report["cases"] if not case["ok"]])))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
