# SPDX-License-Identifier: MIT
"""Check authored Bootstrap card shadows across PDF and static HTML exports.

The independent geometry and Gaussian probes come from smoke_inset_shadows.py.
PDF readers and Pillow are development dependencies, not package dependencies.
"""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path
import runpy

from fullbleed.ui.experimental import Box, Document, Page, Paragraph, Renderer, Theme, css, pt, style, styled
from fullbleed.ui.experimental import bootstrap as bs
from PIL import Image, ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium


geometry = runpy.run_path(str(Path(__file__).with_name("smoke_inset_shadows.py")))
CASES = {case[0]: case for case in geometry["CASES"]}
SCENARIOS = (
    ("negative-hidden", "root", "block"),
    ("negative-hidden-rounded", "root", "block"),
    ("negative-left", "class", "block"),
    ("large-left", "root", "block"),
    ("rounded-offset", "body-part", "block"),
    ("blur-negative", "root", "block"),
    ("blur-negative", "root", "page-start"),
    ("empty-control", "root", "block"),
)
TIMESTAMP = "2026-10-17T00:00:00Z"
TITLE = "Component shadow sample"
HOST_CSS = """@page {size:120pt 90pt;margin:0}
* {box-sizing:border-box;margin:0;padding:0}
html, body {background:white}
.stage {position:absolute;left:0;top:0;width:160px;height:120px;padding:20px}
.shadow-sample {width:120px;height:80px;background:#205b47;box-shadow:none}
"""


def authored(case, route, edge, *, shadow=True):
    """Build the document solely from Python components, including controls."""
    shadow_value = f"inset {case[1]} #01579b" if shadow else "none"
    paint = style(css(f"box-shadow: {shadow_value}"), background="#90caf9", border_radius=case[2])
    Card = styled(bs.Card, width="120px", height="80px", border=0, padding=0,
                  border_radius=0)
    card = Card(Paragraph("Ready", style=style(margin=0, padding="20px 0 0 42px",
                                               font_size="7px", line_height=1)),
                class_name="shadow-sample").with_parts(body=style(padding=0))
    rules = "* {box-sizing:border-box;margin:0;padding:0} html,body {background:white}"
    if route == "body-part":
        card = card.with_parts(body=style(paint, css("flex:none"), width="120px", height="80px", padding=0))
    elif route == "class":
        # A literal class selector must coexist with generated scoped defaults.
        rules += f" .shadow-sample {{background:#90caf9;border-radius:{case[2]};box-shadow:inset {case[1]} #01579b}}"
    else:
        card = card.with_style(paint)
    stage = Box(card, class_name="stage", style=style(css("position:absolute;left:0;top:0"),
                width="160px", height="120px", padding=0 if edge == "page-start" else "20px"))
    return Document(stage, title=TITLE, lang="en", page=Page((pt(120), pt(90)), margin=pt(0)),
                    theme=Theme(text=style(font_family="Inter", font_size="7px")), styles=rules)


def raster(data, dpi):
    with closing(pdfium.PdfDocument(data)) as parsed:
        assert len(parsed) == 1
        with closing(parsed[0]) as page, closing(page.render(scale=dpi / 72)) as bitmap:
            return bitmap.to_pil().convert("RGB")


def native_image(data):
    with Image.open(BytesIO(data)) as image:
        return image.convert("RGB")


def check(out: Path):
    out.mkdir(parents=True, exist_ok=True)
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp=TIMESTAMP)
    records = []
    for name, route, edge in SCENARIOS:
        case = CASES[name]
        key = f"{name}-{route}-{edge}"
        folder = out / key
        folder.mkdir(parents=True, exist_ok=True)
        document = authored(case, route, edge)
        snapshot = document.compile()
        prepared = document.prepare(renderer)
        direct = document.render_pdf(renderer)
        assert direct == prepared.render_pdf() == document.render_pdf(renderer), key
        styled_html = document.to_html(include_styles=True)
        plain_html = document.to_html(include_styles=False)
        assert "<style>" in styled_html and "<style" not in plain_html and " style=" not in plain_html
        assert "shadow-sample" in plain_html and "Ready" in plain_html
        assert "<script" not in styled_html.lower() and "<script" not in plain_html.lower()
        for filename, content in (("styled.html", styled_html), ("unstyled.html", plain_html), ("host.css", HOST_CSS)):
            (folder / filename).write_text(content, encoding="utf-8", newline="\n")
        engine = renderer.engine_for(document)
        exported = bytes(engine.render_pdf(styled_html, ""))
        assert direct == exported, (key, "styled HTML PDF differs")
        hosted = bytes(engine.render_pdf(plain_html, HOST_CSS))
        pdfs = {"document": direct, "styled-export": exported, "host-styled-export": hosted}
        for mode, data in pdfs.items():
            (folder / f"{mode}.pdf").write_bytes(data)
            reader = PdfReader(BytesIO(data))
            assert len(reader.pages) == 1 and "Ready" in reader.pages[0].extract_text(), (key, mode)
            assert reader.metadata.title == TITLE
        previews = prepared.render_image_pages(dpi=96)
        finalized = engine.render_finalized_pdf_image_pages(str(folder / "document.pdf"), 96)
        assert len(previews) == len(finalized) == 1
        views = [("native", native_image(previews[0])), ("finalized", native_image(finalized[0]))]
        views += [(mode + "-pdfium", raster(data, 96)) for mode, data in pdfs.items()]
        checks = []
        for mode, image in views:
            image.save(folder / f"{mode}.png")
            assert image.size == (160, 120), (key, mode, image.size)
            probes = ([dict(x=80, y=60, expected=[32, 91, 71], tolerance=2)]
                      if mode == "host-styled-export-pdfium" else geometry["probes_for"](case, edge))
            observed = [{**p, "actual": list(image.getpixel((p["x"], p["y"])))} for p in probes]
            checks.append(dict(mode=mode, probes=observed,
                ok=all(max(abs(a-b) for a, b in zip(p["expected"], p["actual"])) <= p["tolerance"] for p in observed)))

        clear_regions = []
        if name in ("negative-hidden", "negative-hidden-rounded", "empty-control"):
            reference = authored(case, route, edge, shadow=False)
            (folder / "clear-reference.html").write_text(reference.to_html(include_styles=True), encoding="utf-8", newline="\n")
            reference_pdf = reference.render_pdf(renderer)
            reference_path = folder / "clear-reference.pdf"
            reference_path.write_bytes(reference_pdf)
            comparisons = [
                ("native", native_image(prepared.render_image_pages(dpi=300)[0]),
                 native_image(reference.render_image_pages(renderer, dpi=300)[0])),
                ("finalized", native_image(engine.render_finalized_pdf_image_pages(str(folder / "document.pdf"), 300)[0]),
                 native_image(engine.render_finalized_pdf_image_pages(str(reference_path), 300)[0])),
                ("document-pdfium", raster(direct, 300), raster(reference_pdf, 300)),
                ("styled-export-pdfium", raster(exported, 300), raster(reference_pdf, 300)),
            ]
            for mode, actual, control in comparisons:
                actual.save(folder / f"{mode}-300.png")
                control.save(folder / f"{mode}-300-reference.png")
                assert actual.size == control.size
                assert actual.width in (500, 501) and actual.height in (375, 376)
                diff = ImageChops.difference(actual, control)
                pixels = diff.tobytes()
                different = sum(pixels[i:i+3] != b"\0\0\0" for i in range(0, len(pixels), 3))
                clear_regions.append(dict(mode=mode, dpi=300, region=[0, 0, *actual.size],
                                          differing_pixels=different, ok=different == 0))
        assert document.compile() == snapshot, (key, "source mutated")
        records.append(dict(name=key, route=route, views=checks, clear_regions=clear_regions,
            pdfs={mode: sha256(data).hexdigest() for mode, data in pdfs.items()},
            deterministic=True, styled_export_bytes_equal=True, source_unchanged=True,
            ok=all(c["ok"] for c in checks + clear_regions)))
    report = dict(schema="fullbleed.component_inset_shadows.v1", version=version("fullbleed"),
                  ok=all(record["ok"] for record in records), cases=records,
                  checks=sum(len(record["views"]) + len(record["clear_regions"]) for record in records),
                  scope="Python Bootstrap cards, reusable defaults, class selectors, named parts, static styled/unstyled HTML exports and inset-shadow regression; not complete CSS parity.")
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    report = check(parser.parse_args().out)
    print(json.dumps({"ok": report["ok"], "version": report["version"], "cases": len(report["cases"]),
                      "checks": report["checks"], "failed": [case["name"] for case in report["cases"] if not case["ok"]]}))
    raise SystemExit(0 if report["ok"] else 1)
