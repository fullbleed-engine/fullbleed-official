#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify family defaults against explicit faces with independent PDF/font readers.

Development-only dependencies: pypdf, pypdfium2, fonttools, pillow.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata
from io import BytesIO
import json
from pathlib import Path
import shutil

import fullbleed
from fontTools.ttLib import TTFont
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
FONTS = ROOT / "examples/design_showcase/fonts"
REGULAR = FONTS / "DMSerifDisplay-Regular.ttf"
ITALIC = FONTS / "DMSerifDisplay-Italic.ttf"
FONTFACE = (
    '@font-face {font-family:"Custom Serif";src:local("DMSerifDisplay-Regular");font-style:normal}'
    '@font-face {font-family:"Custom Serif";src:local("DMSerifDisplay-Italic");font-style:italic}'
)


def embedded_faces(page):
    result = set()
    for ref in page["/Resources"]["/Font"].values():
        font = ref.get_object()
        for descendant in font.get("/DescendantFonts", [font]):
            descriptor = descendant.get_object().get("/FontDescriptor")
            assert descriptor is not None, "Expected an embedded face, found a fallback font"
            descriptor = descriptor.get_object()
            assert "/FontFile2" in descriptor, "Expected an embedded TrueType font"
            with TTFont(BytesIO(descriptor["/FontFile2"].get_data())) as parsed:
                result.add(parsed["name"].getDebugName(6))
    return sorted(result)


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    report = {
        "schema": "fullbleed.font_family_smoke.v1", "ok": False,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "versions": {name: metadata.version(name) for name in
                     ["fullbleed", "pypdf", "pypdfium2", "fonttools", "pillow"]},
        "font_sha256": {p.name: sha256(p.read_bytes()).hexdigest() for p in [REGULAR, ITALIC]},
        "cases": [],
    }
    references = {}

    def render(engine, name, family, style, mode="pdf", prefix="", expected=None):
        html = "<p>Type {{name}} with care.</p>"
        css = prefix + (
            '@page {size:440pt 140pt;margin:20pt}'
            f'p {{font-family:"{family}";font-size:22pt;font-style:{style};margin:0}}'
        )
        labels = ["Alpha"] if mode == "pdf" else ["Alpha", "Bravo"]
        source_html = html.replace("{{name}}", labels[0]) if mode == "pdf" else html
        (out / f"{name}.html").write_text(source_html, encoding="utf-8", newline="\n")
        (out / f"{name}.css").write_text(css, encoding="utf-8", newline="\n")
        if mode == "pdf":
            data = bytes(engine.render_pdf(source_html, css))
        else:
            compiled = engine.compile_pdf(html, css)
            if mode == "fixed":
                data = bytes(compiled.render_pdf_bindings({"name": labels}))
            else:
                data = bytes(compiled.render_pdf_reflow_bindings(
                    {"name": labels}, compression="compact" if mode == "compact" else "throughput"))
        path = out / f"{name}.pdf"
        path.write_bytes(data)
        reader = PdfReader(BytesIO(data))
        assert len(reader.pages) == len(labels), name
        faces = [embedded_faces(page) for page in reader.pages]
        assert all(face == [expected] for face in faces), f"{name}: expected {expected}, found {faces}"
        texts = [" ".join(page.extract_text().split()) for page in reader.pages]
        assert texts == [f"Type {label} with care." for label in labels], (name, texts)
        native = [bytes(png) for png in engine.render_finalized_pdf_image_pages(str(path), 96)]
        assert len(native) == len(labels), name
        native_hashes, independent_hashes = [], []
        with pdfium.PdfDocument(data) as document:
            for index, png in enumerate(native):
                (out / f"{name}-{index + 1}.png").write_bytes(png)
                native_hashes.append(sha256(png).hexdigest())
                page = document[index]
                bitmap = page.render(scale=96 / 72)
                image = bitmap.to_pil()
                independent_hashes.append(sha256(image.tobytes()).hexdigest())
                image.save(out / f"{name}-{index + 1}-pdfium.png")
                image.close()
                bitmap.close()
                page.close()
        result = {"name": name, "mode": mode, "faces": faces, "text": texts,
                  "html_sha256": sha256(source_html.encode()).hexdigest(),
                  "css_sha256": sha256(css.encode()).hexdigest(), "labels": labels,
                  "pdf_sha256": sha256(data).hexdigest(), "native_png_sha256": native_hashes,
                  "pdfium_pixels_sha256": independent_hashes}
        report["cases"].append(result)
        return result

    try:
        # Explicit PostScript names are the control for each style and compiled path.
        for mode in ["pdf", "fixed", "reflow", "compact"]:
            for style, font in [("normal", REGULAR), ("italic", ITALIC)]:
                engine = fullbleed.PdfEngine(font_files=[str(font)])
                references[style, mode] = render(engine, f"control-{style}-{mode}", font.stem,
                                                 style, mode, expected=font.stem)
        for loader in ["files", "bundle", "directory"]:
            for order, paths in [("italic-first", [ITALIC, REGULAR]),
                                 ("regular-first", [REGULAR, ITALIC])]:
                if loader == "files":
                    engine = fullbleed.PdfEngine(font_files=[str(p) for p in paths])
                elif loader == "bundle":
                    engine = fullbleed.PdfEngine()
                    bundle = fullbleed.AssetBundle()
                    for path in paths:
                        bundle.add_file(str(path), fullbleed.AssetKind.Font, name=path.stem)
                    engine.register_bundle(bundle)
                else:
                    folder = out / f"fonts-{order}"
                    folder.mkdir()
                    for index, path in enumerate(paths):
                        shutil.copyfile(path, folder / f"{index}-{path.name}")
                    shutil.copyfile(FONTS / "DMSerifDisplay-OFL.txt", folder / "OFL.txt")
                    engine = fullbleed.PdfEngine(font_dirs=[str(folder)])
                modes = ["pdf", "fixed", "reflow", "compact"] if loader != "directory" else ["pdf"]
                for mode in modes:
                    for style, font in [("normal", REGULAR), ("italic", ITALIC)]:
                        name = f"{loader}-{order}-{style}-{mode}"
                        result = render(engine, name, "DM Serif Display", style, mode, expected=font.stem)
                        control = references[style, mode]
                        for key in ["native_png_sha256", "pdfium_pixels_sha256"]:
                            assert result[key] == control[key], f"{name}: differs from explicit face ({key})"
                if loader != "directory":
                    for style, font in [("normal", REGULAR), ("italic", ITALIC)]:
                        name = f"{loader}-{order}-fontface-{style}"
                        result = render(engine, name, "Custom Serif", style, prefix=FONTFACE, expected=font.stem)
                        for key in ["native_png_sha256", "pdfium_pixels_sha256"]:
                            assert result[key] == references[style, "pdf"][key], name
        # Duplicate explicit aliases preserve the first requested face.
        for order, paths in [("italic-first", [ITALIC, REGULAR]),
                             ("regular-first", [REGULAR, ITALIC])]:
            engine = fullbleed.PdfEngine()
            bundle = fullbleed.AssetBundle()
            for path in paths:
                bundle.add_file(str(path), fullbleed.AssetKind.Font, name="Brand")
            engine.register_bundle(bundle)
            render(engine, f"explicit-alias-{order}", "Brand", "normal", expected=paths[0].stem)
        report["ok"] = True
        report["pages"] = sum(len(case["faces"]) for case in report["cases"])
        return report
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({"ok": result["ok"], "cases": len(result["cases"]), "pages": result["pages"]}))
