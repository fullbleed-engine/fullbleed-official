#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare embedded CSS with independently authored explicit-CSS controls.

Exercises installed direct/compiled/fixed/reflow output, mixed-document batch
paths, native previews and independent PDFium pixels. This is a bounded static
stylesheet regression suite, not a general browser or standards-conformance test.
Development-only dependencies: pypdf, pypdfium2 and Pillow.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium

BASE = """@page {size:300pt 200pt;margin:20pt}
html,body {margin:0;padding:0;background:white}
body {font-family:Inter;font-size:10pt;line-height:1.3;color:black}
p {margin:0}
"""
RED = "p {color:#d00000;font-size:18pt}"
BLUE = "p {color:#0033dd;font-size:18pt}"
MODES = ("direct", "compiled", "fixed", "reflow")


def cases():
    def case(name, head, expected="", *, explicit="", source_tail="", control_tail="", attrs=""):
        body = f'<p id="probe" {attrs}>Invoice {{{{number}}}}</p>'
        template = '<!doctype html><html><head>{}</head><body>{}</body></html>'
        return {"name": name, "html": template.format(head, body + source_tail),
                "control": template.format("", body + control_tail),
                "css": BASE + explicit, "control_css": BASE + expected}

    yield case("ordinary", f"<style>{RED}</style>", RED)
    yield case("document-order", f"<style>{RED}</style><style>{BLUE}</style>", BLUE)
    yield case("explicit-last", f"<style>{RED}</style>", BLUE, explicit=BLUE)
    yield case("specificity", "<style>#probe {color:#d00000;font-size:18pt}</style>", RED, explicit=BLUE)
    yield case("important", "<style>p {color:#d00000 !important;font-size:18pt}</style>", RED, explicit="p {color:blue}")
    yield case("inline-wins", f"<style>{RED}</style>", "p {font-size:18pt}", attrs='style="color:#009944"')
    yield case("important-over-inline", "<style>p {color:#d00000 !important;font-size:18pt}</style>",
               "p {color:#d00000 !important;font-size:18pt}", attrs='style="color:#009944"')
    yield case("layers", "<style>@layer first, second; @layer first {" + RED + "}</style><style>@layer second {" + BLUE + "}</style>", BLUE)
    yield case("malformed-isolation", f"<style>/* unclosed</style><style>{RED}</style>", RED)
    yield case("import-ignored", f"<style>@import url('https://example.invalid/never.css'); {RED}</style>", RED)
    yield case("template-inert", f"<template><style>{RED}</style></template>")
    for kind, expected in (("", RED), ("TeXt/CsS", RED), (" text/css ", ""), ("text/css;charset=utf-8", ""), ("text/plain", "")):
        name = {"": "empty", "TeXt/CsS": "mixed", " text/css ": "spaces", "text/css;charset=utf-8": "parameter", "text/plain": "plain"}[kind]
        yield case("type-" + name, f'<style type="{kind}">{RED}</style>', expected)
    for media, expected in (("print", RED), ("screen", ""), ("all", RED), ("not print", ""), ("screen, print", RED), ("print and (min-width: 100px)", RED), ("print and (min-width: 9000px)", "")):
        name = media.replace(" ", "-").replace(",", "").replace(":", "").replace("(", "").replace(")", "")
        yield case("media-" + name, f'<style media="{media}">{RED}</style>', expected)
    yield case("nested-media", f"<style>@media screen {{{BLUE}}} @media print {{{RED}}}</style>", RED)
    yield case("legacy-wrappers", f"<style><!-- {RED} --></style>", RED)
    yield case("named-sheets", f'<style title="first">{RED}</style><style title="second">{BLUE}</style>', RED)
    yield case("preferred-meta", f'<meta http-equiv="default-style" content="second"><style title="first">{RED}</style><style title="second">{BLUE}</style>', BLUE)
    yield case("html-raw-entity", "<style>p::after {content:' &amp;'}</style>", "p::after {content:' &amp;'}")
    for name, content, expected in (("svg-global", RED, RED), ("svg-cdata", f"<![CDATA[{RED}]]>", RED),
                                    ("svg-entity", "p::after {content:' &amp;'}", "p::after {content:' &'}")):
        yield case(name, "", expected,
                   source_tail=f'<svg width="10" height="10"><style>{content}</style></svg>',
                   control_tail='<svg width="10" height="10"></svg>')
    yield case("page-margin-box", "<style>@page {size:A5; @top-center {content:'Invoice header'}}</style>",
               "@page {size:A5; @top-center {content:'Invoice header'}}",
               explicit="@page {size:A5}")
    geometry = case("embedded-page-geometry", "<style>@page {size:A5;margin:20pt}</style>", "@page {size:A5;margin:20pt}")
    geometry["css"] = BASE.split("\n", 1)[1]
    yield geometry


def new_engine(**options):
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    return fullbleed.PdfEngine(font_files=[str(font)], document_timestamp="2026-10-16T00:00:00Z", **options)


def render(engine, html, css, mode):
    labels = ["1042"] if mode in ("direct", "compiled") else ["1042", "1043"]
    if mode == "direct":
        action = lambda: engine.render_pdf(html.replace("{{number}}", labels[0]), css)
    elif mode == "compiled":
        compiled = engine.compile_pdf(html.replace("{{number}}", labels[0]), css)
        action = compiled.render_pdf
    else:
        compiled = engine.compile_pdf(html, css)
        action = (lambda: compiled.render_pdf_bindings({"number": labels})) if mode == "fixed" else (
            lambda: compiled.render_pdf_reflow_bindings({"number": labels}))
    data = bytes(action())
    assert data == bytes(action()), "repeat render changed"
    return data


def inspect_pdf(data, folder, prefix):
    reader = PdfReader(BytesIO(data))
    result = {"text": [" ".join((page.extract_text() or "").split()) for page in reader.pages],
              "boxes": [[float(page.mediabox.width), float(page.mediabox.height)] for page in reader.pages],
              "pixels": []}
    with pdfium.PdfDocument(data) as document:
        for index, page in enumerate(document):
            bitmap = page.render(scale=1)
            image = bitmap.to_pil().convert("RGB")
            result["pixels"].append({"size": list(image.size), "sha256": sha256(image.tobytes()).hexdigest()})
            image.save(folder / f"{prefix}-{index + 1}.png")
            image.close(); bitmap.close(); page.close()
    assert all(result["text"]), "empty PDF text"
    return result


def check_batch_and_reports(out, result):
    folder = out / "batch-and-reports"
    folder.mkdir(exist_ok=True)
    records = [("1042", RED), ("1043", BLUE), ("1044", RED)]
    htmls = [f"<style>{css}</style><p>Invoice {number}</p>" for number, css in records]
    expected = {"text": [], "boxes": [], "pixels": []}
    engine = new_engine()
    for index, (number, css) in enumerate(records):
        observation = inspect_pdf(bytes(engine.render_pdf(f"<p>Invoice {number}</p>", BASE + css)), folder, f"control-{index}")
        for key in expected:
            expected[key].extend(observation[key])

    def verify(name, produce, expected):
        row = {"name": name}
        try:
            data = bytes(produce())
            (folder / (name + ".pdf")).write_bytes(data)
            observation = inspect_pdf(data, folder, name)
            row.update(ok=observation == expected, source=observation, control=expected)
        except Exception as error:
            row.update(ok=False, error=str(error))
        result["cases"].append(row)
        if not row["ok"]:
            result["failures"].append(name)

    for jit in ("off", "plan", "replay"):
        engine = new_engine(jit_mode=jit)
        jobs = [(html, BASE) for html in htmls]
        for method, arguments in (
            ("render_pdf_batch", (htmls, BASE)),
            ("render_pdf_batch_parallel", (htmls, BASE)),
            ("render_pdf_batch_with_css", (jobs,)),
        ):
            verify(f"{method}-{jit}", lambda: getattr(engine, method)(*arguments), expected)
        for method, arguments in (
            ("render_pdf_batch_to_file", (htmls, BASE)),
            ("render_pdf_batch_with_css_to_file", (jobs,)),
            ("render_pdf_batch_to_file_parallel", (htmls, BASE)),
            ("render_pdf_batch_to_file_parallel_with_page_data", (htmls, BASE)),
        ):
            name = f"{method}-{jit}"
            path = folder / (name + ".pdf")
            def produce():
                getattr(engine, method)(*arguments, str(path))
                return path.read_bytes()
            verify(name, produce, expected)
        one = {key: values[:1] for key, values in expected.items()}
        for method in ("render_pdf_with_page_data", "render_pdf_with_glyph_report",
                       "render_pdf_with_page_data_and_glyph_report",
                       "render_pdf_with_page_data_and_template_bindings",
                       "render_pdf_with_page_data_and_template_bindings_and_glyph_report",
                       "render_pdf_with_glyph_report_and_render_time_reading_order_trace"):
            verify(f"{method}-{jit}", lambda: getattr(engine, method)(htmls[0], BASE)[0], one)

    # Bundle sources are independent parse boundaries and precede document sheets.
    engine = new_engine()
    bundle = fullbleed.AssetBundle()
    for name, css in (("incomplete.css", "/* incomplete"), ("base.css", BASE + BLUE)):
        path = folder / name
        path.write_text(css, encoding="utf-8")
        bundle.add_file(str(path), fullbleed.AssetKind.Css, name=name)
    engine.register_bundle(bundle)
    one = {key: values[:1] for key, values in expected.items()}
    verify("bundle-then-embedded", lambda: engine.render_pdf(htmls[0], ""), one)
    blue = inspect_pdf(bytes(new_engine().render_pdf("<p>Invoice 1042</p>", BASE + BLUE)), folder, "blue-control")
    verify("bundle-embedded-then-explicit", lambda: engine.render_pdf(htmls[0], BLUE), blue)

    # Streaming writers must initialize from the first document's actual paper,
    # including when no paper setting appears in the explicit CSS argument.
    geometry_css = BASE.split("\n", 1)[1]
    geometry_htmls = [f"<style>@page {{size:A5;margin:20pt}} {css}</style><p>Invoice {number}</p>"
                      for number, css in records]
    geometry_expected = {"text": [], "boxes": [], "pixels": []}
    for index, (number, css) in enumerate(records):
        observation = inspect_pdf(bytes(new_engine().render_pdf(
            f"<p>Invoice {number}</p>", geometry_css + "@page {size:A5;margin:20pt}" + css)),
            folder, f"geometry-control-{index}")
        for key in geometry_expected:
            geometry_expected[key].extend(observation[key])
    for jit in ("off", "plan", "replay"):
        engine = new_engine(jit_mode=jit)
        for method in ("render_pdf_batch", "render_pdf_batch_parallel", "render_pdf_batch_with_css",
                       "render_pdf_batch_to_file", "render_pdf_batch_with_css_to_file",
                       "render_pdf_batch_to_file_parallel", "render_pdf_batch_to_file_parallel_with_page_data"):
            name = f"embedded-paper-{method}-{jit}"
            arguments = ([(html, geometry_css) for html in geometry_htmls],) if "with_css" in method else (
                geometry_htmls, geometry_css)
            path = folder / (name + ".pdf")
            def produce():
                if "to_file" in method:
                    getattr(engine, method)(*arguments, str(path))
                    return path.read_bytes()
                return getattr(engine, method)(*arguments)
            verify(name, produce, geometry_expected)

    # Structural reflow content uses the template's compiled stylesheet.
    engine = new_engine()
    template = f'<style>{RED}</style><section data-fb-bind-html="content"></section>'
    fragments = ["<p>Invoice 1042</p>", "<p>Invoice 1043</p>"]
    expected = {"text": [], "boxes": [], "pixels": []}
    for index, fragment in enumerate(fragments):
        observation = inspect_pdf(bytes(engine.render_pdf(f"<section>{fragment}</section>", BASE + RED)),
                                  folder, f"structural-control-{index}")
        for key in expected:
            expected[key].extend(observation[key])
    compiled = engine.compile_pdf(template, BASE)
    verify("embedded-styles-in-structural-reflow",
           lambda: compiled.render_pdf_reflow_bindings({"content": fragments}), expected)


def check(out: Path, modes=MODES):
    out.mkdir(parents=True, exist_ok=True)
    result = {"schema": "fullbleed.embedded_styles.v1", "ok": False, "scope": __doc__,
              "runtime": fullbleed.build_features(), "version": metadata.version("fullbleed"),
              "cases": [], "failures": []}
    engine = new_engine()
    for case in cases():
        for mode in modes:
            name = case["name"] + "-" + mode
            folder = out / name
            folder.mkdir(exist_ok=True)
            row = {"name": name}
            try:
                observations = []
                for prefix, html, css in (("source", case["html"], case["css"]), ("control", case["control"], case["control_css"])):
                    (folder / (prefix + ".html")).write_text(html, encoding="utf-8")
                    (folder / (prefix + ".css")).write_text(css, encoding="utf-8")
                    data = render(engine, html, css, mode)
                    (folder / (prefix + ".pdf")).write_bytes(data)
                    observation = inspect_pdf(data, folder, prefix)
                    if mode == "direct":
                        previews = engine.render_image_pages(html.replace("{{number}}", "1042"), css, 72)
                        observation["native"] = [sha256(bytes(png)).hexdigest() for png in previews]
                    observations.append(observation)
                row.update(ok=observations[0] == observations[1], source=observations[0], control=observations[1])
            except Exception as error:
                row.update(ok=False, error=str(error))
            result["cases"].append(row)
            if not row["ok"]:
                result["failures"].append(name)
    if tuple(modes) == MODES:
        check_batch_and_reports(out, result)
    result["ok"] = not result["failures"]
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--direct-only", action="store_true")
    args = parser.parse_args()
    result = check(args.out, ("direct",) if args.direct_only else MODES)
    print(json.dumps({"ok": result["ok"], "cases": len(result["cases"]), "failures": result["failures"]}))
    raise SystemExit(0 if result["ok"] else 1)
