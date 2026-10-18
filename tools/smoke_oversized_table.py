#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check visible text and logical cells when table rows cross page boundaries.

Uses installed Fullbleed plus development-only pypdf/PDFium readers. Retains
inputs, PDFs, bounds, structure checks and preview images. External PDF/UA
validation and human assistive-technology checks remain separate gates.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
import json
import math
from pathlib import Path
import re
import runpy

import fullbleed
import pypdfium2 as pdfium

inspect = runpy.run_path(str(Path(__file__).with_name("smoke_table_tagging.py")))["inspect"]

PROFILES = ("none", "tagged", "pdfua1", "pdfua2")
MODES = ("direct", "compiled", "repeat", "reflow", "compact")
CSS = """@page {size:320pt 160pt;margin:18pt}
body {font-family:Inter;font-size:10pt;line-height:1.3}
h1 {font-size:16pt;margin:0 0 10pt} p {margin:0 0 4pt}
table {width:100%;border-collapse:collapse} th,td {padding:5pt;text-align:left}
.decorated th,.decorated td {border:1pt solid #254d45;background:#e9f1ed}
.bold td {font-weight:700} .accent {color:#a13241}
ul {margin:0;padding-left:14pt} svg {vertical-align:middle}
 td {border:1pt solid #254d45; background:#e9f1ed}"""
CONTENT = "".join(f"<p>LINE{i:03} <strong>Garden together</strong> with neighbors.</p>" for i in range(12))
HEADER = '<thead><tr><th scope="col">Content</th></tr></thead>'
ORIGINAL = f'<h1>Long cell</h1><table>{HEADER}<tbody><tr><td>{CONTENT}</td></tr></tbody></table>'


def documents():
    yield "original-baseline", ORIGINAL, CSS
    yield "without-header", ORIGINAL.replace(HEADER, ""), CSS
    yield "top-aligned", ORIGINAL, CSS + " td {vertical-align:top}"
    yield "fitting-control", ORIGINAL, CSS.replace("320pt 160pt", "320pt 900pt")
    yield "mixed-columns", ("<h1>Long cell</h1><table><thead><tr>"
        "<th id='item' scope='col'>Item</th><th id='detail' scope='col'>Detail</th><th scope='col'>Status</th>"
        "</tr></thead><tbody><tr><td>BEFORE</td><td>Start</td><td></td></tr>"
        "<tr><th id='person' scope='row'>ROW_LABEL</th><td headers='person detail'>"
        + CONTENT + "</td><td>READY</td></tr><tr><td>AFTER</td><td>End</td><td></td></tr></tbody></table>"), CSS.replace("320pt 160pt", "420pt 160pt")
    yield "fragmented-row-header", ("<h1>Long cell</h1><table><thead><tr><th scope='col'>Program</th>"
        "<th scope='col'>Status</th></tr></thead><tbody><tr><th id='program' scope='row'>"
        + CONTENT + "</th><td headers='program'>READY</td></tr></tbody></table>"), CSS
    items = "<ul>" + "".join(f"<li>LINE{i:03} Garden together</li>" for i in range(12)) + "</ul>"
    yield "list-cell", ORIGINAL.replace(CONTENT, items), CSS
    yield "colspan-cell", ("<h1>Long cell</h1><table><thead><tr><th id='a' scope='col'>A</th>"
        "<th id='b' scope='col'>B</th></tr></thead><tbody><tr><td colspan='2' headers='a b'>"
        + CONTENT + "</td></tr></tbody></table>"), CSS
    yield "nonrepeating-label", ORIGINAL.replace(HEADER, "").replace("<tbody>", "<tbody><tr style='break-after:avoid'><th scope='col'>LABEL</th></tr>"), CSS
    yield "repeated-footer", ORIGINAL.replace("</table>", "<tfoot><tr><td>FOOTER</td></tr></tfoot></table>"), CSS
    plain = "\n".join(f"LINE{i:03} Garden together" for i in range(12))
    yield "plain-lines", ORIGINAL.replace(CONTENT, plain), CSS + " td {white-space:pre-line}"
    yield "separate-borders", ORIGINAL, CSS + " table {border-collapse:separate;border-spacing:3pt}"


def visible_pages(data: bytes, folder: Path, save_images: bool) -> list[dict]:
    pages = []
    with pdfium.PdfDocument(data) as pdf:
        for i in range(len(pdf)):
            page = pdf[i]
            text = page.get_textpage()
            outside = []
            glyphs = []
            for char in range(text.count_chars()):
                value = text.get_text_range(char, 1)
                if not value.strip():
                    continue
                left, bottom, right, top = text.get_charbox(char)
                if not (left >= 17 and bottom >= 17 and right <= page.get_width()-17 and top <= page.get_height()-17):
                    outside.append({"char": value, "bounds": [left, bottom, right, top]})
                else:
                    glyphs.append((value, left, bottom, right, top))
            bitmap = page.render(scale=2)
            image = bitmap.to_pil().convert("RGBA")
            missing_ink = []
            # These fixtures use dark text on white or pale cell backgrounds.
            # Text extraction alone also sees glyphs hidden by a clipping path.
            for value, left, bottom, right, top in glyphs:
                crop = image.crop((math.floor(left*2), math.floor((page.get_height()-top)*2),
                    math.ceil(right*2), math.ceil((page.get_height()-bottom)*2)))
                pixels = crop.get_flattened_data() if hasattr(crop, "get_flattened_data") else crop.getdata()
                if not any(sum(pixel[:3]) < 540 for pixel in pixels):
                    missing_ink.append({"char": value, "bounds": [left, bottom, right, top]})
                crop.close()
            if save_images:
                image.save(folder/f"page-{i+1}.png")
            pages.append({"page": i+1, "text": text.get_text_range(), "size": [page.get_width(), page.get_height()],
                "outside_glyphs": outside, "glyphs_without_visible_ink": missing_ink,
                "rgba_sha256": sha256(image.tobytes()).hexdigest()})
            image.close(); bitmap.close(); text.close(); page.close()
    return pages


def check(out: Path, names=None, profiles=PROFILES, modes=MODES) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    font = Path(str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")))
    report = {"schema": "fullbleed.oversized_table_smoke.v1", "ok": False,
        "versions": {name: metadata.version(name) for name in ("fullbleed", "pypdf", "pypdfium2")},
        "font_sha256": sha256(font.read_bytes()).hexdigest(), "cases": []}
    pixel_references = {}
    for profile in profiles:
        engine = fullbleed.PdfEngine(pdf_profile=profile, document_lang="en-US",
            document_title="Long table cell", font_files=[str(font)])
        for name, source, css in documents():
            if names and name not in names:
                continue
            for mode in modes:
                folder = out/profile/name/mode
                folder.mkdir(parents=True)
                html = source.replace("Long cell</h1>", "Long cell {{record}}</h1>") if mode in {"reflow", "compact"} else source
                (folder/"source.html").write_text(html, encoding="utf-8", newline="\n")
                (folder/"source.css").write_text(css, encoding="utf-8", newline="\n")
                errors = []
                compiled = None if mode == "direct" else engine.compile_pdf(html, css)
                def render():
                    if mode == "direct":
                        return bytes(engine.render_pdf(html, css))
                    if mode == "compiled":
                        return bytes(compiled.render_pdf())
                    if mode == "repeat":
                        return bytes(compiled.render_pdf_batch(2))
                    return bytes(compiled.render_pdf_reflow_bindings({"record": ["Ada", "Bea"]},
                        compression="compact" if mode == "compact" else "throughput"))
                try:
                    data = render()
                    (folder/"document.pdf").write_bytes(data)
                    replay = render()
                    if data != replay:
                        (folder/"replay.pdf").write_bytes(replay)
                        errors.append("Repeated rendering changed PDF bytes")
                    pages = visible_pages(data, folder, profile in {"none", "pdfua1"} and mode == "direct")
                    text = "\n".join(page["text"] for page in pages)
                    records = 1 if mode in {"direct", "compiled"} else 2
                    for i in range(12):
                        if text.count(f"LINE{i:03}") != records:
                            errors.append(f"Missing or duplicated LINE{i:03}")
                    if any(page["outside_glyphs"] for page in pages):
                        errors.append("Visible glyphs extend outside the page content box")
                    if any(page["glyphs_without_visible_ink"] for page in pages):
                        errors.append("Extracted glyph has no visible ink at its reported position")
                    if name != "fitting-control" and len(pages) <= records:
                        errors.append("Oversized cell did not paginate")
                    for page in pages:
                        if "Content" in page["text"] and "LINE" not in page["text"]:
                            errors.append("Repeated header without any body content")
                        if not any(marker in page["text"] for marker in ("LINE", "BEFORE", "AFTER")):
                            errors.append("Page has no table body content")
                    if name == "mixed-columns":
                        for marker in ["BEFORE", "ROW_LABEL", "READY", "AFTER"]:
                            if text.count(marker) != records:
                                errors.append(f"Missing or repeated companion: {marker}")
                    result = {"pages": pages, "pdf_sha256": sha256(data).hexdigest()}
                    pixels = [page["rgba_sha256"] for page in pages]
                    key = (name, mode)
                    if profile == "none":
                        pixel_references[key] = pixels
                    elif key in pixel_references and pixels != pixel_references[key]:
                        errors.append("Tagging changed rendered pixels")
                    if profile != "none":
                        structure = inspect(data)
                        errors.extend(structure["errors"])
                        for role in ["Table", "TR", "TH", "TD"]:
                            expected = len(re.findall(r"<"+role.lower()+r"(?=[\s>])", html)) * records
                            if structure["roles"].get("/"+role, 0) != expected:
                                errors.append(f"Logical {role} count differs from authored cells/rows")
                        expected_links = len(re.findall(r"\bheaders=", html))*records
                        if sum(bool(cell["headers"]) for cell in structure["cells"]) != expected_links:
                            errors.append("Explicit cell header relationship lost")
                        result["structure"] = structure
                except Exception as error:
                    result = {}
                    errors.append(f"{type(error).__name__}: {error}")
                result.update(name=name, profile=profile, mode=mode, ok=not errors, errors=errors)
                report["cases"].append(result)
                (out/"verification.json").write_text(json.dumps(report, indent=2)+"\n", encoding="utf-8")
    report["ok"] = bool(report["cases"]) and all(case["ok"] for case in report["cases"])
    (out/"verification.json").write_text(json.dumps(report, indent=2)+"\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--case", action="append")
    parser.add_argument("--profile", choices=PROFILES, action="append")
    parser.add_argument("--mode", choices=MODES, action="append")
    args = parser.parse_args()
    report = check(args.out, args.case, args.profile or PROFILES, args.mode or MODES)
    print(json.dumps({"ok": report["ok"], "cases": len(report["cases"]),
        "failed": [{key: case[key] for key in ("name", "profile", "mode", "errors")} for case in report["cases"] if not case["ok"]]}))
    raise SystemExit(0 if report["ok"] else 1)
