#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check that deferred filters preserve ancestor overflow clips and z-order.

Checks interior colors in native previews and finalized PDFs. --pdfium also
uses an independent PDF renderer. These are targeted regressions, not a claim
of general CSS parity. Browser-verified expectations follow:
https://www.w3.org/TR/css-overflow-3/#overflow-properties
https://www.w3.org/TR/filter-effects-1/#FilterProperty
"""
from __future__ import annotations

import argparse
import hashlib
from importlib import metadata
import json
from pathlib import Path
import runpy
import urllib.parse

WHITE, RED, BLUE, GREEN = [255, 255, 255], [255, 0, 0], [0, 0, 255], [0, 255, 0]
BASE = """@page { size: 240px 180px; margin: 0; }
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body { background: white; }
.parent { position: absolute; left: 20px; top: 20px; width: 100px; height: 50px;
  overflow: hidden; border: 4px solid #003366; border-radius: 12px; background: #ddeeff; }
.child { width: 60px; height: 70px; margin: 20px 10px 0; background: red; filter: contrast(1.1); }
"""
HTML = '<!doctype html><html><body><div class="parent"><div class="child"></div></div></body></html>'


def cases() -> list[dict]:
    result = []

    def add(name, css, probes, html=HTML):
        result.append(dict(name=name, html=html, css=BASE + css,
                           probes=[dict(x=x, y=y, expected=rgb) for x, y, rgb in probes]))

    for name, effect, overflow, radius in [
        ("unfiltered-control", "none", "hidden", 0),
        ("contrast-hidden", "contrast(1.1)", "hidden", 0),
        ("contrast-rounded", "contrast(1.1)", "hidden", 12),
        ("blur-rounded", "blur(2px)", "hidden", 12),
        ("shadow-rounded", "drop-shadow(4px 3px 0 #123456)", "hidden", 12),
        ("visible-control", "contrast(1.1)", "visible", 0),
    ]:
        add(name, f".parent {{ overflow: {overflow}; border-radius: {radius}px; }} .child {{ filter: {effect}; }}",
            [(50, 55, RED), (50, 85, WHITE if overflow == "hidden" else RED)])
    add("nested-clips", """
        .parent { width: 100px; height: 60px; border: 0; border-radius: 0; background: lime; }
        .inner { width: 50px; height: 60px; margin: 20px; overflow: hidden; background: blue; }
        .child { width: 80px; height: 80px; margin: 10px; }
        """, [(60, 60, RED), (100, 60, GREEN), (60, 90, WHITE)],
        HTML.replace('<div class="child"></div>', '<div class="inner"><div class="child"></div></div>'))
    add("transformed-parent", ".parent { transform: scale(1.5); transform-origin: 0 0; }",
        [(65, 65, RED), (65, 120, WHITE)])
    add("transformed-child", ".child { transform: translate(20px, 10px); }",
        [(70, 60, RED), (70, 95, WHITE)])
    add("rounded-path", """
        .parent { height: 80px; border-radius: 24px; }
        .child { width: 120px; height: 100px; margin: 0; }
        """, [(25, 25, WHITE), (34, 25, [0, 51, 102]), (40, 30, RED), (60, 110, WHITE)])
    add("vertical-clip-only", """
        .parent { overflow: visible clip; border-radius: 0; }
        .child { width: 140px; }
        """, [(50, 55, RED), (150, 55, RED), (50, 85, WHITE)])
    add("clip-margin", ".parent { overflow: clip; overflow-clip-margin: 8px; border-radius: 0; }",
        [(50, 55, RED), (50, 72, RED), (50, 85, WHITE)])
    # Overflow alone must not create a stacking context: the red child (z=10)
    # remains above its blue uncle (z=1), but is clipped below its parent.
    add("ancestor-z-order", """
        .child { position: relative; z-index: 10; }
        .uncle { position: absolute; left: 30px; top: 50px; width: 70px;
          height: 45px; background: blue; z-index: 1; }
        """, [(50, 55, RED), (50, 85, BLUE)],
        HTML.replace('</body>', '<div class="uncle"></div></body>'))
    add("positive-vector-clip", ".child { position: relative; z-index: 10; filter: none; }",
        [(50, 55, RED), (50, 85, WHITE)])
    add("parent-stacking-context", """
        .parent { z-index: 1; }
        .child { position: relative; z-index: 10; }
        .uncle { position: absolute; left: 30px; top: 50px; width: 70px;
          height: 45px; background: blue; z-index: 2; }
        """, [(50, 55, BLUE), (50, 85, BLUE)],
        HTML.replace('</body>', '<div class="uncle"></div></body>'))
    svg = '<svg xmlns="http://www.w3.org/2000/svg" width="60" height="70"><rect width="60" height="70" fill="red"/></svg>'
    image = '<img class="child" alt="" src="data:image/svg+xml,' + urllib.parse.quote(svg, safe="") + '">'
    add("filtered-image", ".child { display: block; background: none; }",
        [(50, 55, RED), (50, 85, WHITE)],
        HTML.replace('<div class="child"></div>', image))
    return result


def check(output: Path, *, pdfium: bool = False) -> dict:
    import fullbleed

    decode_png = runpy.run_path(str(Path(__file__).with_name("run_css_fixture_suite.py")))["_decode_png_rgba_rows"]
    if pdfium:
        import pypdfium2
    output.mkdir(parents=True, exist_ok=True)
    engine = fullbleed.PdfEngine()
    report = dict(schema="fullbleed.filtered_overflow.v1", ok=False,
                  version=metadata.version("fullbleed"), cases=[],
                  scope="Ancestor clip, transform, and z-order regressions; not general CSS parity.")

    def png_view(folder, mode, pages, probes, expected_pages):
        views = []
        assert len(pages) == expected_pages, (folder.name, mode, len(pages))
        for index, data in enumerate(pages):
            png = bytes(data)
            (folder / f"{mode}-{index + 1}.png").write_bytes(png)
            width, height, rows = decode_png(png)
            assert (width, height) == (240, 180), (folder.name, mode, width, height)
            measured = [dict(**probe, actual=list(rows[probe["y"]][probe["x"] * 4:probe["x"] * 4 + 3])) for probe in probes]
            views.append(dict(mode=mode, page=index + 1, probes=measured))
        return views

    def pdf_views(folder, name, data, probes, expected_pages=1):
        path = folder / f"{name}.pdf"
        path.write_bytes(data)
        views = png_view(folder, name + "-finalized", list(engine.render_finalized_pdf_image_pages(str(path), 96)), probes, expected_pages)
        if pdfium:
            with pypdfium2.PdfDocument(data) as document:
                assert len(document) == expected_pages, (folder.name, name, len(document))
                for index in range(len(document)):
                    page = document[index]
                    try:
                        bitmap = page.render(scale=96 / 72)
                        try:
                            image = bitmap.to_pil().convert("RGB")
                            assert image.size == (240, 180), (folder.name, name, image.size)
                            image.save(folder / f"{name}-pdfium-{index + 1}.png")
                            views.append(dict(mode=name + "-pdfium", page=index + 1, probes=[
                                dict(**probe, actual=list(image.getpixel((probe["x"], probe["y"])))) for probe in probes]))
                        finally:
                            bitmap.close()
                    finally:
                        page.close()
        return dict(mode=name, sha256=hashlib.sha256(data).hexdigest(), pages=expected_pages, views=views)

    for case in cases():
        name, html, css, probes = (case[key] for key in ("name", "html", "css", "probes"))
        folder = output / name
        folder.mkdir(exist_ok=True)
        (folder / "source.html").write_text(html, encoding="utf-8")
        (folder / "source.css").write_text(css, encoding="utf-8")
        data = bytes(engine.render_pdf(html, css))
        documents = [pdf_views(folder, "direct", data, probes),
                     pdf_views(folder, "compiled", bytes(engine.compile_pdf(html, css).render_pdf()), probes)]
        views = png_view(folder, "native", list(engine.render_image_pages(html, css, 96)), probes, 1)
        if name == "contrast-rounded":
            marker_html = html.replace('</body>', '<p class="record">Record {{record}}</p></body>')
            marker_css = css + ".record { position: absolute; left: 20px; top: 150px; font: 12px Helvetica; }"
            compiled = engine.compile_pdf(marker_html, marker_css)
            (folder / "bindings.html").write_text(marker_html, encoding="utf-8")
            (folder / "bindings.css").write_text(marker_css, encoding="utf-8")
            for mode, pdf in [
                ("batch", engine.compile_pdf(html, css).render_pdf_batch(2)),
                ("fixed-bindings", compiled.render_pdf_bindings({"record": ["A", "B"]})),
                ("reflow-bindings", compiled.render_pdf_reflow_bindings({"record": ["A", "B"]})),
            ]:
                documents.append(pdf_views(folder, mode, bytes(pdf), probes, 2))
        for document in documents:
            views.extend(document["views"])
        for view in views:
            view["ok"] = all(max(abs(a - b) for a, b in zip(probe["expected"], probe["actual"])) <= 3 for probe in view["probes"])
        deterministic = data == bytes(engine.render_pdf(html, css))
        report["cases"].append(dict(name=name, documents=[{k: v for k, v in doc.items() if k != "views"} for doc in documents],
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
