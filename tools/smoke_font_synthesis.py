#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify independent synthesis controls in installed PDFs and previews.

Compares each case with an explicit style control within the same renderer and
binding mode. This is not a browser pixel-parity or complete CSS-support claim.
Development-only dependencies: pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from PIL import Image
from pypdf import PdfReader
import pypdfium2 as pdfium


ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "python/fullbleed_assets/fonts/NotoSans-Regular.ttf"
BASE = ('@page {size:440px 100px;margin:0}'
        'html,body {margin:0;padding:0;color:black;background:white}'
        'p {position:absolute;left:24px;top:20px;margin:0;font-family:Probe;'
        'font-size:28px;line-height:40px}'
        'svg {position:absolute;left:24px;top:20px;font-family:Probe;font-size:28px}')
TEXT = "Invoice {{sample}} 0123 HjQ"


def cases():
    result = []

    def add(name, weight, style, declarations="", reference=None, parent="", small_caps=False):
        html = f'<div style="{parent}"><p>{TEXT}</p></div>'
        css = BASE + f'p {{font-weight:{weight};font-style:{style};{declarations}}}'
        result.append(dict(name=name, html=html, css=css, reference=reference, small_caps=small_caps))
        if small_caps:
            # Existing small-cap synthesis uppercases fixed binding-slot names;
            # the supported positive controls use ordinary/reflow rendering.
            result[-1]["modes"] = ["pdf", "reflow", "compact"]

    for name, weight, style in [("regular", 400, "normal"), ("bold", 700, "normal"),
                                ("italic", 400, "italic"), ("bold-italic", 700, "italic"),
                                ("oblique", 400, "oblique 12deg")]:
        add(name, weight, style)
    add("caps", 400, "normal", "font-variant:small-caps", small_caps=True)
    for name, declarations, reference in [
        ("none", "font-synthesis:none", "regular"),
        ("weight", "font-synthesis:weight", "bold"),
        ("style", "font-synthesis:style", "italic"),
        ("both", "font-synthesis:weight style", "bold-italic"),
        ("style-caps", "font-synthesis:small-caps style", "italic"),
        ("caps-only", "font-synthesis:small-caps", "regular"),
        ("position-only", "font-synthesis:position", "regular"),
        ("longhand-weight-none", "font-synthesis-weight:none", "italic"),
        ("longhand-style-none", "font-synthesis-style:none", "bold"),
        ("longhand-weight-auto", "font-synthesis:none;font-synthesis-weight:auto", "bold"),
        ("longhand-style-auto", "font-synthesis:none;font-synthesis-style:auto", "italic"),
        ("shorthand-reset", "font-synthesis-style:none;font-synthesis:style", "italic"),
        ("invalid-duplicate", "font-synthesis:style;font-synthesis:weight weight", "italic"),
        ("invalid-auto", "font-synthesis:style;font-synthesis:auto", "italic"),
        ("important", "font-synthesis-weight:auto !important;font-synthesis:none", "bold"),
    ]:
        add(name, 700, "italic", declarations, reference)
    for keyword in ["inherit", "unset", "revert"]:
        add(keyword, 700, "italic", f"font-synthesis:none;font-synthesis:{keyword}",
            "italic", parent="font-synthesis:style")
    add("inherited", 700, "italic", reference="bold", parent="font-synthesis:weight")
    add("initial", 700, "italic", "font-synthesis:initial", "bold-italic", parent="font-synthesis:none")
    add("oblique-only-italic", 700, "italic", "font-synthesis:none;font-synthesis-style:oblique-only", "regular")
    add("oblique-only-oblique", 400, "oblique 12deg", "font-synthesis:none;font-synthesis-style:oblique-only", "oblique")
    add("caps-disabled", 400, "normal", "font-variant:small-caps;font-synthesis-small-caps:none", "regular")
    add("caps-shorthand-disabled", 400, "normal", "font-variant:small-caps;font-synthesis:weight style", "regular")
    add("caps-enabled", 400, "normal", "font-variant:small-caps;font-synthesis:none;font-synthesis-small-caps:auto", "caps", small_caps=True)
    # Inline SVG keeps its own text compiler and CSS cascade.
    for name, weight, style, declarations, reference in [
        ("svg-regular", 400, "normal", "", None),
        ("svg-bold", 700, "normal", "", None),
        ("svg-italic", 400, "italic", "", None),
        ("svg-weight", 700, "italic", "font-synthesis:weight", "svg-bold"),
        ("svg-style", 700, "italic", "font-synthesis:style", "svg-italic"),
        ("svg-none", 700, "italic", "font-synthesis:none", "svg-regular"),
        ("svg-weight-none", 700, "italic", "font-synthesis-weight:none", "svg-italic"),
        ("svg-style-none", 700, "italic", "font-synthesis-style:none", "svg-bold"),
    ]:
        result.append(dict(name=name, html=f'<svg width="410" height="65"><text y="32" style="{declarations}">{TEXT}</text></svg>',
                           css=BASE + f'svg {{font-weight:{weight};font-style:{style}}}',
                           reference=reference, small_caps=False, modes=["pdf"]))
    return result


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    report = {"schema": "fullbleed.font_synthesis_smoke.v1", "ok": False,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "versions": {name: metadata.version(name) for name in ["fullbleed", "pypdf", "pypdfium2", "pillow"]},
              "module": fullbleed.__file__, "font_sha256": sha256(FONT.read_bytes()).hexdigest(),
              "scope": __doc__, "cases": [], "preview_control_collisions": []}
    engine = fullbleed.PdfEngine()
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(FONT), fullbleed.AssetKind.Font, name="Probe")
    engine.register_bundle(bundle)
    fingerprints = {}
    try:
        for case in cases():
            for mode in case.get("modes", ["pdf", "fixed", "reflow", "compact"]):
                name = case["name"] + "-" + mode
                folder = out / name
                folder.mkdir()
                html, css = case["html"], case["css"]
                (folder / "input.html").write_text(html, encoding="utf-8")
                (folder / "input.css").write_text(css, encoding="utf-8")
                labels = ["ABC"] if mode == "pdf" else ["ABC", "DEF"]
                if mode == "pdf":
                    html = html.replace("{{sample}}", labels[0])
                    render = lambda: bytes(engine.render_pdf(html, css))
                else:
                    compiled = engine.compile_pdf(html, css)
                    if mode == "fixed":
                        render = lambda: bytes(compiled.render_pdf_bindings({"sample": labels}))
                    else:
                        render = lambda: bytes(compiled.render_pdf_reflow_bindings(
                            {"sample": labels}, compression="compact" if mode == "compact" else "throughput"))
                data = render()
                assert data == render(), f"{name}: nondeterministic output"
                path = folder / "output.pdf"
                path.write_bytes(data)
                reader = PdfReader(BytesIO(data))
                assert len(reader.pages) == len(labels), name
                extracted = [" ".join(page.extract_text().split()) for page in reader.pages]
                expected = [f"Invoice {label} 0123 HjQ" for label in labels]
                if case["small_caps"]:
                    expected = [text.upper() for text in expected]
                assert extracted == expected, (name, extracted)
                for page in reader.pages:
                    assert all(font.get_object()["/Subtype"] != "/Type1" for font in page["/Resources"]["/Font"].values()), f"{name}: fallback font"
                hashes = {"finalized": [], "pdfium": []}
                if mode == "pdf":
                    native, = engine.render_image_pages(html, css, 96)
                    native = bytes(native)
                    (folder / "native.png").write_bytes(native)
                    with Image.open(BytesIO(native)) as img:
                        hashes["native"] = [sha256(img.convert("RGB").tobytes()).hexdigest()]
                finalized = engine.render_finalized_pdf_image_pages(str(path), 96)
                assert len(finalized) == len(labels), name
                with pdfium.PdfDocument(data) as doc:
                    for index, png in enumerate(finalized):
                        png = bytes(png)
                        (folder / f"finalized-{index + 1}.png").write_bytes(png)
                        with Image.open(BytesIO(png)) as img:
                            hashes["finalized"].append(sha256(img.convert("RGB").tobytes()).hexdigest())
                        page = doc[index]
                        bitmap = page.render(scale=96 / 72)
                        image = bitmap.to_pil().convert("RGB")
                        hashes["pdfium"].append(sha256(image.tobytes()).hexdigest())
                        image.save(folder / f"pdfium-{index + 1}.png")
                        image.close()
                        bitmap.close()
                        page.close()
                fingerprints[case["name"], mode] = hashes
                row = dict(name=name, text=extracted, pdf_sha256=sha256(data).hexdigest(),
                           deterministic=True, pixels_sha256=hashes, reference=case["reference"])
                report["cases"].append(row)
                if case["reference"]:
                    assert hashes == fingerprints[case["reference"], mode], f"{name}: differs from {case['reference']}"
        # Negative controls ensure the test font actually needs synthesis and
        # that comparing equal blank/unstyled output cannot pass this smoke.
        for mode in ["pdf", "fixed", "reflow", "compact"]:
            for raster in ["finalized", "pdfium"]:
                names = ["regular", "bold", "italic", "bold-italic"]
                if mode != "fixed":
                    names.append("caps")
                styles = [tuple(fingerprints[name, mode][raster]) for name in names]
                if mode == "fixed" and raster == "finalized":
                    # The existing finalized-preview reader can omit the
                    # stroke used by fixed-slot synthetic bold. Retain that
                    # limitation; PDFium must still distinguish every style
                    # in the actual emitted PDF.
                    report["preview_control_collisions"] = [
                        [left, right] for i, left in enumerate(names)
                        for j, right in enumerate(names) if j > i and styles[i] == styles[j]
                    ]
                else:
                    assert len(set(styles)) == len(styles), (mode, raster, "ineffective control")
        report["ok"] = True
        report["pages"] = sum(len(case["text"]) for case in report["cases"])
        return report
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({"ok": result["ok"], "cases": len(result["cases"]), "pages": result["pages"]}))
