#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare page-margin font aliases with explicit faces in installed PDF output.

Uses existing, visually distinct font fixtures as markers for CSS descriptor
selection. A 700 descriptor intentionally selects the supplied display face;
this checks source selection without relying on variable-font interpolation.
Development dependencies: pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib.metadata import version
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
FONTS = [
    (ROOT / "python/fullbleed_assets/fonts/NotoSans-Regular.ttf", "Body.ttf"),
    (ROOT / "examples/design_showcase/fonts/DMSerifDisplay-Regular.ttf", "Display.ttf"),
]
BASE = '''
@page { size: 320pt 240pt; margin: 40pt 24pt;
  @bottom-center { content: "FOOTER " counter(page) " of " counter(pages);
                   font-size: 12pt; line-height: 18pt; }
}
html { font-family: "Noto Sans"; font-size: 14pt; font-synthesis: none; }
body, p { margin: 0; }
'''
HTML = '<p>Record {{name}} — body text.</p>'
DISPLAY = '@font-face {font-family:Brand;src:url("Display.ttf");font-weight:400;}'
CASES = [
    {"name": "local-alias", "alias": '@font-face {font-family:Brand;src:local("DMSerifDisplay-Regular");} @page {@bottom-center {font-family:Brand;}}',
     "control": '@page {@bottom-center {font-family:"DM Serif Display";}}'},
    {"name": "url-alias", "alias": DISPLAY + r'@page {@bottom-center {font-family:Br\61 nd;}}',
     "control": '@page {@bottom-center {font-family:"DM Serif Display";}}'},
    {"name": "inherited-weight", "alias": '@font-face {font-family:Brand;src:url("Body.ttf");font-weight:400;} @font-face {font-family:Brand;src:url("Display.ttf");font-weight:700;} html {font-family:Brand;} @page {font-weight:700;}',
     "control": '@page {font-family:"DM Serif Display";font-weight:700;}'},
    {"name": "fallback-alias", "alias": DISPLAY + '@page {@bottom-center {font-family:"Unavailable",Brand;}}',
     "control": '@page {@bottom-center {font-family:"Unavailable","DM Serif Display";}}'},
    {"name": "page-inherit", "alias": DISPLAY + '@page {font-family:Brand;@bottom-center {font-family:inherit;}}',
     "control": '@page {font-family:"DM Serif Display";}'},
    {"name": "quoted-generic-alias", "alias": '@font-face {font-family:"serif";src:url("Display.ttf");} @page {font-family:"serif";}',
     "control": '@page {font-family:"DM Serif Display";}'},
    {"name": "size-adjust", "alias": '@font-face {font-family:Adjusted;src:url("Display.ttf");size-adjust:125%;} @page {font-family:Adjusted;}',
     "control": '@page {font-family:"DM Serif Display";@bottom-center {font-size:15pt;}}'},
    {"name": "named-pages", "alias": DISPLAY + '@font-face {font-family:BodyAlias;src:url("Body.ttf");} @page:first {font-family:Brand;} @page second {font-family:BodyAlias;} .second {break-before:page;page:second;}',
     "control": '@page:first {font-family:"DM Serif Display";} @page second {font-family:"Noto Sans";} .second {break-before:page;page:second;}',
     "html": '<section><p>Record {{name}} — first body.</p></section><section class="second"><p>Record {{name}} — second body.</p></section>'},
]


def inspect_pdf(data: bytes):
    reader = PdfReader(BytesIO(data), strict=True)
    pages = []
    for page in reader.pages:
        spans = []
        page.extract_text(visitor_text=lambda text, cm, tm, font, size: spans.append({
            "text": " ".join(text.split()), "font": str(font["/BaseFont"]).split("+")[-1], "size": size,
        }) if text.strip() and font else None)
        footers = [span for span in spans if span["text"].startswith("FOOTER")]
        assert len(footers) == 1, spans
        assert footers[0]["font"] in ["DMSerifDisplay-Regular", "NotoSans-Regular"], footers
        fonts = []
        for reference in page["/Resources"]["/Font"].values():
            font = reference.get_object()
            descendants = font.get("/DescendantFonts", [font])
            for descendant in descendants:
                descriptor = descendant.get_object().get("/FontDescriptor")
                assert descriptor is not None, "Unexpected unembedded fallback: " + str(font["/BaseFont"])
                descriptor = descriptor.get_object()
                assert any(key in descriptor for key in ["/FontFile", "/FontFile2", "/FontFile3"])
            fonts.append(str(font["/BaseFont"]).split("+")[-1])
        pages.append({"text": " ".join(page.extract_text().split()), "footer": footers[0], "fonts": sorted(fonts)})
    return pages


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    report = {"schema": "fullbleed.page_margin_fonts.v1", "ok": False,
              "versions": {name: version(name) for name in ["fullbleed", "pypdf", "pypdfium2", "pillow"]},
              "fontSha256": {alias: sha256(path.read_bytes()).hexdigest() for path, alias in FONTS}, "cases": []}

    def render(case, kind, mode):
        folder = out / (case["name"] + "-" + mode) / kind
        folder.mkdir(parents=True)
        engine = fullbleed.PdfEngine()
        bundle = fullbleed.AssetBundle()
        for path, alias in FONTS:
            bundle.add_file(str(path), fullbleed.AssetKind.Font, name=alias)
        engine.register_bundle(bundle)
        html = case.get("html", HTML)
        css = BASE + case[kind]
        input_html = html.replace("{{name}}", "Alpha") if mode == "pdf" else html
        (folder / "input.html").write_text(input_html, encoding="utf8", newline="\n")
        (folder / "input.css").write_text(css, encoding="utf8", newline="\n")
        if mode == "pdf":
            data = bytes(engine.render_pdf(input_html, css))
        else:
            compiled = engine.compile_pdf(html, css)
            bindings = {"name": ["Alpha", "Bravo"]}
            (folder / "bindings.json").write_text(json.dumps(bindings) + "\n", encoding="utf8", newline="\n")
            if mode == "fixed":
                data = bytes(compiled.render_pdf_bindings(bindings))
            else:
                data = bytes(compiled.render_pdf_reflow_bindings(
                    bindings, compression="compact" if mode == "compact" else "throughput"))
        path = folder / "document.pdf"
        path.write_bytes(data)
        pages = inspect_pdf(data)
        expected = (2 if case["name"] == "named-pages" else 1) * (1 if mode == "pdf" else 2)
        assert len(pages) == expected
        assert all("Record" in page["text"] and "body" in page["text"] for page in pages)
        assert "Alpha" in pages[0]["text"] and (mode == "pdf" or "Bravo" in pages[-1]["text"])
        assert pages[0]["footer"]["font"] == "DMSerifDisplay-Regular"
        if case["name"] == "named-pages":
            assert pages[1]["footer"]["font"] == "NotoSans-Regular"
        assert pages[0]["footer"]["size"] == (15.0 if case["name"] == "size-adjust" else 12.0)
        native = [bytes(image) for image in engine.render_finalized_pdf_image_pages(str(path), 96)]
        assert len(native) == len(pages)
        pixels = []
        with pdfium.PdfDocument(data) as document:
            for number, (page, png) in enumerate(zip(document, native), 1):
                (folder / f"page-{number}.png").write_bytes(png)
                textpage = page.get_textpage()
                text = " ".join(textpage.get_text_range().split())
                assert text == pages[number - 1]["text"]
                width, height = page.get_size()
                for index in range(textpage.count_chars()):
                    char = textpage.get_text_range(index, 1)
                    if not char or char.isspace():
                        continue
                    left, bottom, right, top = textpage.get_charbox(index)
                    assert min(left, bottom) >= -0.25 and right <= width + 0.25 and top <= height + 0.25
                image = page.render(scale=96 / 72).to_pil().convert("RGB")
                image.save(folder / f"page-{number}-pdfium.png")
                pixels.append(sha256(image.tobytes()).hexdigest())
                image.close()
                textpage.close()
                page.close()
        return {"pdfSha256": sha256(data).hexdigest(), "pages": pages,
                "nativePreviewSha256": [sha256(png).hexdigest() for png in native], "pdfiumPixelsSha256": pixels}

    try:
        for case in CASES:
            for mode in ["pdf", "fixed", "reflow", "compact"]:
                report["activeCase"] = {"name": case["name"], "mode": mode}
                control = render(case, "control", mode)
                alias = render(case, "alias", mode)
                for key in ["pages", "nativePreviewSha256", "pdfiumPixelsSha256"]:
                    assert alias[key] == control[key], (case["name"], mode, key)
                report["cases"].append({"name": case["name"], "mode": mode, "control": control, "alias": alias,
                                        "matchesExplicitFaces": True})
        report["ok"] = True
        del report["activeCase"]
        report["pairs"] = len(report["cases"])
        report["pages"] = sum(len(case["alias"]["pages"]) * 2 for case in report["cases"])
        return report
    except Exception as exc:
        report["error"] = {"type": type(exc).__name__, "message": str(exc)}
        raise
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf8", newline="\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({key: result[key] for key in ["ok", "pairs", "pages"]}))
