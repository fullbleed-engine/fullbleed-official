#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check inline row spacing and pagination in saved PDFs using PDFium.

Development-only dependencies: pypdfium2, pillow. Fonts come from the installed
Fullbleed wheel. No browser, system fonts, network or frozen screenshots required.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib import metadata, resources
import json
from pathlib import Path

import fullbleed
import pypdfium2 as pdfium


VARIANTS = ("plain", "span", "color", "nested", "before", "after")


def rows(variant: str, count: int, prefix: str = "") -> str:
    result = []
    for index in range(count):
        marker = f"ROW{prefix}{index:03}"
        content = {
            "plain": f"a {marker}",
            "span": f"<span>a </span>{marker}",
            "color": f"<span class='accent'>a </span>{marker}",
            "nested": f"<span><span class='accent'>a</span> </span>{marker}",
            "before": marker,
            "after": marker,
        }[variant]
        result.append(f"<div class='{variant}'>{content}</div>")
    return "".join(result)


def stylesheet(font: str, size: int, factor: float, width: int, height: int) -> str:
    return (
        f"@page {{size:{width}px {height}px;margin:0}} "
        "* {margin:0;padding:0} "
        f"html,body {{font-family:'{font}';font-size:{size}px;line-height:{factor}}} "
        ".accent,.before::before,.after::after {color:#bc4749} "
        ".before::before {content:'a '} .after::after {content:' a'}"
    )


def inspect(data: bytes, markers: list[str], folder: Path, *, preview: bool) -> tuple[int, dict]:
    """Map each marker's first glyph to its page and top coordinate in points."""
    positions = {marker: [] for marker in markers}
    with pdfium.PdfDocument(data) as document:
        for page_index in range(len(document)):
            page = document[page_index]
            try:
                text = page.get_textpage()
                try:
                    glyphs = [(char, index) for index in range(text.count_chars())
                              for char in text.get_text_range(index, 1) if not char.isspace()]
                    compact = "".join(char for char, _ in glyphs)
                    for marker in markers:
                        if marker not in compact:
                            continue
                        assert compact.count(marker) == 1, f"Repeated {marker} on page {page_index + 1}"
                        glyph = glyphs[compact.index(marker)][1]
                        top = page.get_height() - text.get_charbox(glyph)[3]
                        positions[marker].append((page_index, top))
                finally:
                    text.close()
                if preview:
                    bitmap = page.render(scale=1.5)
                    try:
                        image = bitmap.to_pil()
                        try:
                            image.save(folder / f"pdfium-{page_index + 1}.png")
                        finally:
                            image.close()
                    finally:
                        bitmap.close()
            finally:
                page.close()
        page_count = len(document)
    assert all(len(values) == 1 for values in positions.values()), "Missing or repeated row markers"
    return page_count, {marker: values[0] for marker, values in positions.items()}


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    fonts = [Path(str(resources.files("fullbleed_assets").joinpath("fonts", name)))
             for name in ("NotoSans-Regular.ttf", "Inter-Variable.ttf")]
    engine = fullbleed.PdfEngine(font_files=[str(font) for font in fonts])
    report = {"schema": "fullbleed.inline_line_height_smoke.v1", "ok": False,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "versions": {name: metadata.version(name) for name in ("fullbleed", "pypdfium2", "pillow")},
              "font_sha256": {font.name: sha256(font.read_bytes()).hexdigest() for font in fonts},
              "cases": []}

    def render(name, html, css, markers, mode, preview=False):
        folder = out / name
        folder.mkdir()
        (folder / "source.html").write_text(html, encoding="utf-8", newline="\n")
        (folder / "source.css").write_text(css, encoding="utf-8", newline="\n")
        if mode == "direct":
            data = bytes(engine.render_pdf(html, css))
        else:
            # Reflow goes through the compiled/bound-document path with the
            # same text and line geometry; each call produces one record.
            template = html.replace("ROW", "{{prefix}}")
            compiled = engine.compile_pdf(template, css)
            data = bytes(compiled.render_pdf_reflow_bindings({"prefix": ["ROW"]}))
        (folder / "document.pdf").write_bytes(data)
        pages, positions = inspect(data, markers, folder, preview=preview)
        return {"name": name, "mode": mode, "pdf_sha256": sha256(data).hexdigest(),
                "pages": pages, "positions": positions, "ok": False}

    try:
        for mode in ("direct", "reflow"):
            for font in ("Noto Sans", "Inter"):
                for size in (12, 16, 20, 26, 42):
                    for factor in (1, 1.5):
                        name = f"{mode}-{font.replace(' ', '-')}-{size}-{factor}"
                        panels = "".join(
                            f"<section style='left:{20 + column * 330}px'>{rows(variant, 8, str(column))}</section>"
                            for column, variant in enumerate(VARIANTS)
                        )
                        markers = [f"ROW{column}{index:03}" for column in range(6) for index in range(8)]
                        css = stylesheet(font, size, factor, 2000, 900)
                        css += "section {position:absolute;top:20px;width:310px}"
                        case = render(name, panels, css, markers, mode,
                                      preview=mode == "direct" and size == 20 and factor == 1)
                        report["cases"].append(case)
                        errors = [abs(case["positions"][f"ROW{column}{index:03}"][1]
                                      - case["positions"][f"ROW{column}000"][1]
                                      - index * size * factor * .75)
                                  for column in range(6) for index in range(8)]
                        case["max_advance_error_pt"] = max(errors)
                        case["ok"] = case["pages"] == 1 and max(errors) < .01
                size = 20 if font == "Noto Sans" else 12
                baseline = None
                for variant in VARIANTS:
                    name = f"{mode}-{font.replace(' ', '-')}-pagination-{variant}"
                    markers = [f"ROW{index:03}" for index in range(24)]
                    case = render(name, rows(variant, 24), stylesheet(font, size, 1, 500, size * 8),
                                  markers, mode, preview=mode == "direct" and variant == "color")
                    report["cases"].append(case)
                    if baseline is None:
                        baseline = case["positions"]
                    positions_match = all(
                        case["positions"][marker][0] == baseline[marker][0]
                        and abs(case["positions"][marker][1] - baseline[marker][1]) < .01
                        for marker in markers
                    )
                    case["ok"] = case["pages"] == 3 and positions_match
        report["ok"] = all(case["ok"] for case in report["cases"])
        assert report["ok"], "Inline line-height regression: " + ", ".join(
            case["name"] for case in report["cases"] if not case["ok"])
        return report
    finally:
        (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps({"ok": result["ok"], "cases": len(result["cases"])}))
