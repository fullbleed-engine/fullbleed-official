from __future__ import annotations

import json
from pathlib import Path
from typing import Iterable

import fitz
import fullbleed

from components.fb_ui import el, render_node


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
OUT.mkdir(parents=True, exist_ok=True)

TEMPLATE_PDF = OUT / "rgb_template_3pages.pdf"
RAW_OVERLAY_PDF = OUT / "overlay_raw.pdf"
EL_OVERLAY_PDF = OUT / "overlay_el.pdf"
RAW_COMPOSED_PDF = OUT / "composed_raw.pdf"
EL_COMPOSED_PDF = OUT / "composed_el.pdf"
REPORT_JSON = OUT / "smoke_report.json"

PAGE_SEQUENCE = [
    "blue",
    "red",
    "green",
    "blue",
    "green",
    "red",
    "blue",
    "red",
    "green",
    "blue",
]

TEMPLATE_ID_BY_COLOR = {
    "blue": "tpl-blue",
    "red": "tpl-red",
    "green": "tpl-green",
}
TEMPLATE_PAGE_BY_ID = {
    "tpl-blue": 0,
    "tpl-red": 1,
    "tpl-green": 2,
}

PAGE_SIZE = (612, 792)  # Letter portrait in points


def build_template_pdf(path: Path) -> None:
    doc = fitz.open()
    square = fitz.Rect(206, 296, 406, 496)
    palette = [("blue", (0, 0, 1)), ("red", (1, 0, 0)), ("green", (0, 1, 0))]
    for color_name, rgb in palette:
        page = doc.new_page(width=PAGE_SIZE[0], height=PAGE_SIZE[1])
        page.draw_rect(
            square,
            color=rgb,
            fill=rgb,
            width=0,
            overlay=True,
        )
        page.insert_text(
            fitz.Point(222, 270),
            f"TEMPLATE::{color_name.upper()}",
            fontsize=18,
            color=(0, 0, 0),
        )
    doc.save(path)
    doc.close()


def common_css() -> str:
    return """
@page { size: 8.5in 11in; margin: 0.6in; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; color: #111; }
.page {
  min-height: 9.8in;
  display: block;
  position: relative;
}
.page:not(:last-child) { break-after: page; }
.meta-table { width: 100%; border-collapse: collapse; margin-bottom: 8pt; }
.meta-table td { font-size: 1pt; color: transparent; line-height: 1; }
.banner { font-size: 18pt; font-weight: 700; margin: 0 0 8pt 0; }
.subtitle { font-size: 11pt; margin: 0 0 12pt 0; }
.body-copy { font-size: 10pt; line-height: 1.35; max-width: 6.6in; }
.field-grid { margin-top: 14pt; font-size: 10pt; border-collapse: collapse; width: 100%; }
.field-grid td { border: 1px solid #333; padding: 4pt 6pt; }
""".strip()


def raw_html(sequence: Iterable[str]) -> str:
    chunks: list[str] = []
    for idx, color in enumerate(sequence, start=1):
        feature = f"fb.feature.{color}=1"
        chunks.append(
            f"""
<section class="page page-{color}">
  <table class="meta-table"><tbody><tr data-fb="{feature}"><td>meta</td></tr></tbody></table>
  <h1 class="banner">RAW MODE PAGE {idx:02d}</h1>
  <p class="subtitle">feature={color} template={TEMPLATE_ID_BY_COLOR[color]}</p>
  <p class="body-copy">
    This page exercises feature-driven template binding from raw HTML. Rendering should preserve text layout,
    table borders, and pagination while finalize composes the correct background template page.
  </p>
  <table class="field-grid">
    <tbody>
      <tr><td>Record</td><td>{idx:02d}</td></tr>
      <tr><td>Color</td><td>{color}</td></tr>
      <tr><td>TemplateId</td><td>{TEMPLATE_ID_BY_COLOR[color]}</td></tr>
    </tbody>
  </table>
</section>
""".strip()
        )
    return "<!doctype html><html><body>" + "\n".join(chunks) + "</body></html>"


def el_html(sequence: Iterable[str]) -> str:
    pages = []
    for idx, color in enumerate(sequence, start=1):
        feature = f"fb.feature.{color}=1"
        page = el(
            "section",
            el(
                "table",
                el("tbody", el("tr", el("td", "meta"), **{"data-fb": feature})),
                class_name="meta-table",
            ),
            el("h1", f"EL MODE PAGE {idx:02d}", class_name="banner"),
            el("p", f"feature={color} template={TEMPLATE_ID_BY_COLOR[color]}", class_name="subtitle"),
            el(
                "p",
                "This page exercises feature-driven template binding from the el() wrapper path.",
                class_name="body-copy",
            ),
            el(
                "table",
                el(
                    "tbody",
                    el("tr", el("td", "Record"), el("td", f"{idx:02d}")),
                    el("tr", el("td", "Color"), el("td", color)),
                    el("tr", el("td", "TemplateId"), el("td", TEMPLATE_ID_BY_COLOR[color])),
                ),
                class_name="field-grid",
            ),
            class_name=f"page page-{color}",
        )
        pages.append(page)
    return "<!doctype html><html><body>" + "".join(render_node(p) for p in pages) + "</body></html>"


def build_engine() -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        template_binding={
            "default_template_id": "tpl-blue",
            "by_feature": {
                "blue": "tpl-blue",
                "red": "tpl-red",
                "green": "tpl-green",
            },
            "feature_prefix": "fb.feature.",
        }
    )


def expected_template_ids() -> list[str]:
    return [TEMPLATE_ID_BY_COLOR[c] for c in PAGE_SEQUENCE]


def render_overlay_and_bindings(mode: str, html: str, css: str, out_pdf: Path) -> list[dict]:
    engine = build_engine()
    pdf_bytes, _page_data, bindings = engine.render_pdf_with_page_data_and_template_bindings(html, css)
    out_pdf.write_bytes(pdf_bytes)
    if not isinstance(bindings, list) or len(bindings) != len(PAGE_SEQUENCE):
        raise RuntimeError(f"{mode}: missing or invalid bindings payload")

    got = [b.get("template_id") for b in bindings]
    exp = expected_template_ids()
    if got != exp:
        raise RuntimeError(f"{mode}: template bindings mismatch expected={exp} got={got}")
    return bindings


def compose_from_bindings(bindings: list[dict], overlay_pdf: Path, out_pdf: Path) -> dict:
    templates = [
        ("tpl-blue", str(TEMPLATE_PDF)),
        ("tpl-red", str(TEMPLATE_PDF)),
        ("tpl-green", str(TEMPLATE_PDF)),
    ]
    plan = []
    for b in bindings:
        template_id = b["template_id"]
        page_index = int(b["page_index"])
        plan.append(
            (
                template_id,
                TEMPLATE_PAGE_BY_ID[template_id],
                page_index,
                0.0,
                0.0,
            )
        )
    return fullbleed.finalize_compose_pdf(templates, plan, str(overlay_pdf), str(out_pdf))


def sample_template_color(pdf_path: Path, page_index: int) -> str:
    doc = fitz.open(pdf_path)
    page = doc[page_index]
    pix = page.get_pixmap(matrix=fitz.Matrix(1, 1), alpha=False)
    x = pix.width // 2
    y = pix.height // 2
    r, g, b = pix.pixel(x, y)
    doc.close()
    if b > 180 and r < 100 and g < 140:
        return "blue"
    if r > 180 and g < 120 and b < 120:
        return "red"
    if g > 160 and r < 140 and b < 140:
        return "green"
    return f"unknown({r},{g},{b})"


def validate_output(pdf_path: Path, mode_label: str) -> dict:
    doc = fitz.open(pdf_path)
    try:
        if doc.page_count != len(PAGE_SEQUENCE):
            raise RuntimeError(
                f"{mode_label}: page count mismatch expected={len(PAGE_SEQUENCE)} got={doc.page_count}"
            )
        color_hits = []
        text_checks = []
        for i, expected_color in enumerate(PAGE_SEQUENCE):
            observed_color = sample_template_color(pdf_path, i)
            color_hits.append({"page": i + 1, "expected": expected_color, "observed": observed_color})
            if observed_color != expected_color:
                raise RuntimeError(
                    f"{mode_label}: page {i+1} expected template color {expected_color} got {observed_color}"
                )
            text = doc[i].get_text("text")
            has_marker = f"PAGE {i+1:02d}" in text
            text_checks.append({"page": i + 1, "has_overlay_marker": has_marker})
            if not has_marker:
                raise RuntimeError(f"{mode_label}: missing overlay marker text on page {i+1}")
        return {
            "ok": True,
            "page_count": doc.page_count,
            "color_checks": color_hits,
            "text_checks": text_checks,
        }
    finally:
        doc.close()


def main() -> None:
    build_template_pdf(TEMPLATE_PDF)
    css = common_css()

    raw_bindings = render_overlay_and_bindings("raw", raw_html(PAGE_SEQUENCE), css, RAW_OVERLAY_PDF)
    raw_compose = compose_from_bindings(raw_bindings, RAW_OVERLAY_PDF, RAW_COMPOSED_PDF)
    raw_validation = validate_output(RAW_COMPOSED_PDF, "raw")

    el_bindings = render_overlay_and_bindings("el", el_html(PAGE_SEQUENCE), css, EL_OVERLAY_PDF)
    el_compose = compose_from_bindings(el_bindings, EL_OVERLAY_PDF, EL_COMPOSED_PDF)
    el_validation = validate_output(EL_COMPOSED_PDF, "el")

    report = {
        "schema": "fullbleed.template_flagging_smoke.v1",
        "ok": True,
        "template_pdf": str(TEMPLATE_PDF),
        "sequence": PAGE_SEQUENCE,
        "raw": {
            "overlay_pdf": str(RAW_OVERLAY_PDF),
            "composed_pdf": str(RAW_COMPOSED_PDF),
            "compose": raw_compose,
            "validation": raw_validation,
        },
        "el": {
            "overlay_pdf": str(EL_OVERLAY_PDF),
            "composed_pdf": str(EL_COMPOSED_PDF),
            "compose": el_compose,
            "validation": el_validation,
        },
    }
    REPORT_JSON.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))


if __name__ == "__main__":
    main()
