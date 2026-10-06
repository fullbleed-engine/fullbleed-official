#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check mixed inline wrapping with independent readers and retained previews.

Development-only dependencies: pypdf, pypdfium2, pillow.
"""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path
import re

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium


TAIL = "to change the paper size, margins, heading scale, table colors, or code treatment. Keep local image references next to the document."
EXPECTED = f"Edit print.css {TAIL} After the paragraph."
CASES = [
    ("plain", "<p>Edit {{file}} " + TAIL + "</p>", ""),
    ("unstyled-span", "<p>Edit <span>{{file}}</span> " + TAIL + "</p>", ""),
    ("code", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", ""),
    ("color", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", "code {color:#ab391c}"),
    ("background", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", "code {background:#eee9da}"),
    ("padding", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", "code {padding:1pt 3pt}"),
    ("different-font", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", "code {font-family:'Noto Sans';font-size:9pt}"),
    ("decorated", "<p>Edit <code>{{file}}</code> " + TAIL + "</p>", "code {font-family:'Noto Sans';font-size:9pt;padding:1pt 3pt;background:#eee9da}"),
    ("long-inline", "<p>Edit <span>{{file}} " + TAIL + "</span></p>", "span {color:#ab391c}"),
    ("collapsed-spaces", "<p>  Edit <span> {{file}} </span>  " + TAIL.replace(" ", "  ") + "  </p>", "span {background:#eee9da}"),
]


def inspect_page(page):
    """Retain per-word ink bounds in PDF coordinates, with y increasing upwards."""
    textpage = page.get_textpage()
    try:
        chars = [textpage.get_text_range(i, 1) for i in range(textpage.count_chars())]
        text = "".join(chars)
        words = []
        for match in re.finditer(r"\S+", text):
            boxes = [textpage.get_charbox(i) for i in range(match.start(), match.end())]
            words.append({"text": match.group(), "box": [min(b[0] for b in boxes),
                         min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes)]})
        return " ".join(text.split()), words
    finally:
        textpage.close()


def visual_order(words):
    rows = []
    for word in sorted(words, key=lambda w: -(w["box"][1] + w["box"][3]) / 2):
        center = (word["box"][1] + word["box"][3]) / 2
        if not rows or abs(rows[-1][0] - center) >= 5:
            rows.append((center, []))
        rows[-1][1].append(word)
    return [word for _, row in rows for word in sorted(row, key=lambda w: w["box"][0])]


def check(out: Path):
    out.mkdir(parents=True, exist_ok=False)
    fonts = [str(resources.files("fullbleed_assets").joinpath("fonts", name))
             for name in ["Inter-Variable.ttf", "NotoSans-Regular.ttf"]]
    report = {"schema": "fullbleed.inline_wrapping_smoke.v1", "ok": False,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "versions": {name: metadata.version(name) for name in ["fullbleed", "pypdf", "pypdfium2", "pillow"]},
              "font_sha256": {Path(p).name: sha256(Path(p).read_bytes()).hexdigest() for p in fonts},
              "cases": []}
    try:
        for mode in ["pdf", "fixed", "reflow", "compact"]:
            for name, source, rule in CASES:
                folder = out / f"{mode}-{name}"
                folder.mkdir()
                # The fixed lane intentionally keeps template geometry. Give
                # its variable an authored slot wider than the replacement.
                html = source.replace("{{file}}", "{{filename}}") + "<p>After the paragraph.</p>"
                css = ("@page {size:240pt 420pt;margin:20pt} body {font-family:Inter;font-size:10pt;line-height:15pt} "
                       "* {margin:0;padding:0} p {margin-bottom:12pt} " + rule)
                (folder / "source.html").write_text(html, encoding="utf-8", newline="\n")
                (folder / "source.css").write_text(css, encoding="utf-8", newline="\n")
                result = {"name": name, "mode": mode, "ok": False}
                report["cases"].append(result)
                try:
                    engine = fullbleed.PdfEngine(font_files=fonts)
                    if mode == "pdf":
                        data = bytes(engine.render_pdf(html.replace("{{filename}}", "print.css"), css))
                    else:
                        compiled = engine.compile_pdf(html, css)
                        bindings = {"filename": ["print.css"]}
                        data = bytes(compiled.render_pdf_bindings(bindings) if mode == "fixed" else
                                     compiled.render_pdf_reflow_bindings(bindings, compression=
                                         "compact" if mode == "compact" else "throughput"))
                    path = folder / "document.pdf"
                    path.write_bytes(data)
                    result["pdf_sha256"] = sha256(data).hexdigest()
                    reader = PdfReader(BytesIO(data))
                    result["pages"] = len(reader.pages)
                    result["pypdf_text"] = " ".join(" ".join(p.extract_text() for p in reader.pages).split())
                    if name in ["different-font", "decorated"]:
                        result["pdf_fonts"] = [str(ref.get_object().get("/BaseFont", ""))
                                               for p in reader.pages for ref in p["/Resources"]["/Font"].values()]
                        assert any("NotoSans-Regular" in face for face in result["pdf_fonts"]), "The second font was not selected"
                    native = list(engine.render_finalized_pdf_image_pages(str(path), 96))
                    for i, png in enumerate(native, 1):
                        (folder / f"native-{i}.png").write_bytes(bytes(png))
                    with pdfium.PdfDocument(data) as document:
                        assert len(document) == len(native) == 1, "Expected a one-page specimen"
                        page = document[0]
                        try:
                            result["pdfium_text"], words = inspect_page(page)
                            result["words"] = words
                            bitmap = page.render(scale=1.5)
                            image = bitmap.to_pil()
                            image.save(folder / "pdfium-1.png")
                            image.close()
                            bitmap.close()
                        finally:
                            page.close()
                    if mode == "fixed":
                        # Fixed templates paint replacements over a static base.
                        # Their content-stream order is not the visual order;
                        # check single-copy content and physical positions here.
                        for key in ["pypdf_text", "pdfium_text"]:
                            assert Counter(result[key].split()) == Counter(EXPECTED.split()), f"{key} content differs"
                        words = visual_order(words)
                        result["visual_words"] = words
                    else:
                        assert result["pypdf_text"] == EXPECTED, "pypdf text order or content differs"
                        assert result["pdfium_text"] == EXPECTED, "PDFium text order or content differs"
                    assert [word["text"] for word in words] == EXPECTED.split(), "Word boundaries differ"
                    prefix, code, suffix = [word["box"] for word in words[:3]]
                    assert prefix[2] <= code[0] and code[2] <= suffix[0], "Initial inline words overlap"
                    assert max(b[1] for b in [prefix, code, suffix]) - min(b[1] for b in [prefix, code, suffix]) < 4, "Following text left the first line"
                    line_count = 1
                    for previous, current in zip(words, words[1:]):
                        a, b = previous["box"], current["box"]
                        if abs((a[1] + a[3]) / 2 - (b[1] + b[3]) / 2) < 5:
                            assert b[0] >= a[2] - .1, f"Overlapping words: {previous['text']} / {current['text']}"
                        else:
                            assert b[3] < a[1], f"Lines overlap or move upwards: {previous['text']} / {current['text']}"
                            line_count += 1
                    assert line_count >= 4, "The fixture did not exercise wrapping"
                    assert all(19 <= w["box"][0] < w["box"][2] <= 221 for w in words), "Text leaves the content box"
                    result["lines"] = line_count
                    result["ok"] = True
                except Exception as error:
                    result["error"] = str(error)
        report["ok"] = all(case["ok"] for case in report["cases"])
        failures = [f"{c['mode']}/{c['name']}: {c['error']}" for c in report["cases"] if not c["ok"]]
        assert report["ok"], "Inline wrapping regression: " + "; ".join(failures)
        return report
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({"ok": result["ok"], "cases": len(result["cases"])}))
