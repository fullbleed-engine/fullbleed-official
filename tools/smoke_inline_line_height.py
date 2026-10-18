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
# Layout baselines measured in Chrome 155 with these exact bundled fonts.
# The native test independently checks the Noto Sans metrics. PDF baselines
# below also exercise terminal painting, inline composition and compiled reflow.
BROWSER_METRICS = json.loads((Path(__file__).resolve().parents[1]
                             / "tests/fixtures/inline-line-height/chrome-metrics.json").read_text(encoding="utf-8"))
UNIT_BASELINES_PX, ROOMY_BASELINES_PX = (
    {font: {row["size_px"]: row["baseline_px"] for row in BROWSER_METRICS["cases"]
            if row["font"] == font and row["factor"] == factor}
     for font in BROWSER_METRICS["font_sha256"]}
    for factor in (1, 1.5)
)


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


def inspect(data: bytes, markers: list[str], folder: Path, *, preview: bool) -> tuple[int, dict, list]:
    """Map each marker's first glyph to its page and top coordinate in points."""
    positions = {marker: [] for marker in markers}
    baselines = []
    with pdfium.PdfDocument(data) as document:
        for page_index in range(len(document)):
            page = document[page_index]
            try:
                baselines.append(sorted({round(page.get_height() - obj.get_matrix().f, 3)
                                         for obj in page.get_objects() if obj.type == pdfium.raw.FPDF_PAGEOBJ_TEXT}))
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
    return page_count, {marker: values[0] for marker, values in positions.items()}, baselines


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    fonts = [Path(str(resources.files("fullbleed_assets").joinpath("fonts", name)))
             for name in ("NotoSans-Regular.ttf", "Inter-Variable.ttf")]
    for family, path in zip(("Noto Sans", "Inter"), fonts):
        assert sha256(path.read_bytes()).hexdigest() == BROWSER_METRICS["font_sha256"][family], (
            f"Refresh the explicit {family} font and browser baseline reference together")
    engine = fullbleed.PdfEngine(font_files=[str(font) for font in fonts])
    report = {"schema": "fullbleed.inline_line_height_smoke.v1", "ok": False,
              "checked_at": datetime.now(timezone.utc).isoformat(),
              "versions": {name: metadata.version(name) for name in ("fullbleed", "pypdfium2", "pillow")},
              "font_sha256": {font.name: sha256(font.read_bytes()).hexdigest() for font in fonts},
              "baseline_reference": BROWSER_METRICS,
              "cases": []}

    def render(name, html, css, markers, mode, preview=False, binding_marker="ROW"):
        folder = out / name
        folder.mkdir()
        (folder / "source.html").write_text(html, encoding="utf-8", newline="\n")
        (folder / "source.css").write_text(css, encoding="utf-8", newline="\n")
        if mode == "direct":
            data = bytes(engine.render_pdf(html, css))
        else:
            # Reflow goes through the compiled/bound-document path with the
            # same text and line geometry; each call produces one record.
            template = html.replace(binding_marker, "{{prefix}}")
            compiled = engine.compile_pdf(template, css)
            data = bytes(compiled.render_pdf_reflow_bindings({"prefix": [binding_marker]}))
        (folder / "document.pdf").write_bytes(data)
        pages, positions, baselines = inspect(data, markers, folder, preview=preview)
        return {"name": name, "mode": mode, "pdf_sha256": sha256(data).hexdigest(),
                "pages": pages, "positions": positions, "baselines_pt": baselines, "ok": False}

    try:
        for mode in ("direct", "reflow"):
            for font in ("Noto Sans", "Inter"):
                for size in UNIT_BASELINES_PX[font]:
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
                        baselines = UNIT_BASELINES_PX if factor == 1 else ROOMY_BASELINES_PX
                        expected = [(20 + baselines[font][size] + row * size * factor) * .75
                                    for row in range(8)]
                        actual = case["baselines_pt"][0]
                        case["expected_baselines_pt"] = expected
                        case["max_baseline_error_pt"] = (
                            max(abs(a - b) for a, b in zip(actual, expected))
                            if len(actual) == len(expected) else None
                        )
                        case["ok"] &= (case["max_baseline_error_pt"] is not None
                                       and case["max_baseline_error_pt"] < .01)
                # The parent's positive odd leading determines the descent
                # beneath a larger child. Check the emitted border, not just
                # text positions: terminal snapping can hide a half-pixel
                # logical error from otherwise-correct foreground glyphs.
                name = f"{mode}-{font.replace(' ', '-')}-mixed-size"
                css = stylesheet(font, 20, 1.5, 400, 120)
                css += ".mixed {width:300px;border-bottom:2px solid #000} .large {font-size:48px;line-height:1}"
                case = render(name, "<div class='mixed'><span class='large'>ROW000</span></div>",
                              css, ["ROW000"], mode)
                report["cases"].append(case)
                with pdfium.PdfDocument((out / name / "document.pdf").read_bytes()) as document:
                    page = document[0]
                    try:
                        borders = []
                        for obj in page.get_objects():
                            if obj.type != pdfium.raw.FPDF_PAGEOBJ_PATH:
                                continue
                            left, bottom, right, top = obj.get_bounds()
                            if abs(right - left - 225) < .01 and abs(top - bottom - 1.5) < .01:
                                borders.append([page.get_height() - top, page.get_height() - bottom])
                    finally:
                        page.close()
                line_height = next(row["height_css_px"] - 2 for row in BROWSER_METRICS["mixed_size"]
                                   if row["family"] == font)
                expected_border = [line_height * .75, (line_height + 2) * .75]
                case["border_bounds_pt"] = borders
                case["expected_border_bounds_pt"] = expected_border
                case["ok"] = (case["pages"] == 1 and len(borders) == 1
                              and max(abs(a - b) for a, b in zip(borders[0], expected_border)) < .01)
                # A logical baseline and its final paint phase are distinct.
                # Whole-pixel origins hide a premature half-leading floor;
                # these saved browser PDFs expose it at quarter-pixel origins.
                for reference in BROWSER_METRICS["fractional_origins"]:
                    if reference["font"] != font:
                        continue
                    top = reference["top_px"]
                    size = reference["size_px"]
                    name = f"{mode}-{font.replace(' ', '-')}-fractional-{top}"
                    panels = "".join(
                        f"<section style='left:{20 + column * 330}px'>{rows(variant, 1, str(column))}</section>"
                        for column, variant in enumerate(VARIANTS)
                    )
                    css = stylesheet(font, size, 1.5, 2000, 200)
                    css += f"section {{position:absolute;top:{top}px;width:310px}}"
                    assert sha256(panels.encode()).hexdigest() == reference["html_sha256"]
                    assert sha256(css.encode()).hexdigest() == reference["css_sha256"]
                    case = render(name, panels, css, [f"ROW{column}000" for column in range(6)],
                                  mode, preview=mode == "direct" and top == 20.5)
                    report["cases"].append(case)
                    expected = reference["paint_baseline_pt"]
                    actual = case["baselines_pt"][0]
                    case["expected_baselines_pt"] = [expected]
                    case["max_baseline_error_pt"] = max(abs(value - expected) for value in actual)
                    case["ok"] = (case["pages"] == 1 and len(actual) == 1
                                  and case["max_baseline_error_pt"] < .01)
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
        for mode in ("direct", "reflow"):
            for reference in BROWSER_METRICS["integral_origins"]:
                family, size, factor, weight, top = (
                    reference[key] for key in ("font", "size_px", "factor", "weight", "top_px"))
                html = f"""<!doctype html><html><head><style>
@page{{size:400px 200px;margin:0}}*{{margin:0;padding:0}}
html,body{{font-family:'{family}';font-size:{size}px;font-weight:{weight};line-height:{factor}}}
.t{{position:absolute;top:{top}px;left:20px;width:360px}}
</style></head><body><div class="t">Hxy</div></body></html>"""
                assert sha256(html.encode()).hexdigest() == reference["html_sha256"]
                case = render(f"{mode}-integral-{reference['id']}", html, "", ["Hxy"], mode,
                              binding_marker="Hxy")
                report["cases"].append(case)
                expected = reference["paint_baseline_pt"]
                actual = case["baselines_pt"][0]
                case["expected_baselines_pt"] = [expected]
                case["max_baseline_error_pt"] = max(abs(value - expected) for value in actual)
                case["ok"] = (case["pages"] == 1 and len(actual) == 1
                              and case["max_baseline_error_pt"] < .01)
            for reference in BROWSER_METRICS["fractional_mixed_origins"]:
                family, top = reference["family"], reference["top"]
                html = f"""<!doctype html><html><head><style>
@page{{size:400px 200px;margin:0}}*{{margin:0;padding:0}}
html,body{{font-family:'{family}';font-size:20px;line-height:1.5}}
.mixed{{position:absolute;top:{top}px;left:20px;width:300px;border-bottom:2px solid #000}}
.large{{font-size:48px;line-height:1}}
</style></head><body><div class="mixed"><span class="large">ROW000</span></div></body></html>"""
                assert sha256(html.encode()).hexdigest() == reference["source_sha256"]
                case = render(f"{mode}-fractional-mixed-{reference['id']}", html, "", ["ROW000"], mode)
                report["cases"].append(case)
                expected = reference["paint_baseline_pt"]
                actual = case["baselines_pt"][0]
                case["expected_baselines_pt"] = [expected]
                case["max_baseline_error_pt"] = max(abs(value - expected) for value in actual)
                case["ok"] = (case["pages"] == 1 and len(actual) == 1
                              and case["max_baseline_error_pt"] < .01)
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
