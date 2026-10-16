# SPDX-License-Identifier: MIT
"""Verify component style ownership against rendered controls from an installed wheel."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui import StyleSheet
from fullbleed.ui.experimental import (
    Card, Document, Flow, Heading, Page, Paragraph, Renderer, Theme,
    css, mm, pt, style, styled, token,
)
from PIL import ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Install the candidate wheel first"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-16T00:00:00Z")
    theme = Theme({"accent": "#315d9b"}, text=style(font_family="Inter", font_size=pt(12), line_height=1.2))
    page = Page("A5", margin=mm(15))
    records = []

    def document(node, rules=None):
        return Document(Flow(Heading("A considered record", level=1), node, gap=pt(18)),
                        title="Component style contract", lang="en-US", page=page, theme=theme, styles=rules)

    def check(name, actual, control, rules=None):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        authored, expected = document(actual, rules), document(control)
        plan = authored.compile(a11y_mode="raise")
        rendered = authored.prepare(renderer, a11y_mode="raise")
        pdf, reference = rendered.render_pdf(), expected.render_pdf(renderer, a11y_mode="raise")
        assert pdf == authored.render_pdf(renderer), (name, "repeat output")
        _, missing = renderer.engine_for(authored).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not missing, (name, missing)
        reader = PdfReader(BytesIO(pdf))
        assert len(reader.pages) == 1, name
        box = reader.pages[0].mediabox
        assert abs(float(box.width) - 419.528) < .01 and abs(float(box.height) - 595.276) < .01
        assert "PAIDINFULL" in "".join(reader.pages[0].extract_text().split())
        assert reader.metadata.title == authored.title
        assert reader.trailer["/Root"]["/Lang"] == authored.lang
        with closing(pdfium.PdfDocument(pdf)) as actual_pdf, closing(pdfium.PdfDocument(reference)) as expected_pdf:
            with closing(actual_pdf[0]) as actual_page, closing(expected_pdf[0]) as expected_page:
                with closing(actual_page.render(scale=1.5)) as actual_bitmap, closing(expected_page.render(scale=1.5)) as expected_bitmap:
                    image, expected_image = actual_bitmap.to_pil().convert("RGB"), expected_bitmap.to_pil().convert("RGB")
                    assert image.size == expected_image.size
                    assert ImageChops.difference(image, expected_image).getbbox() is None, (name, "control pixels")
                    image.save(folder / "independent.png")
        authored.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "control.pdf").write_bytes(reference)
        (folder / "native.png").write_bytes(rendered.render_image_pages(dpi=96)[0])
        records.append({"name": name, "pdf_sha256": sha256(pdf).hexdigest(),
                        "control_pdf_sha256": sha256(reference).hexdigest(), "control_pixels_equal": True,
                        "repeat_bytes_equal": True, "missing_glyphs": 0, "pages": 1,
                        "page_size_pt": [float(box.width), float(box.height)]})

    Label = styled(Paragraph, font_size=pt(18), color="#a0442e")
    base = Label("PAID IN FULL", class_name="notice")
    blue = Label("PAID IN FULL").with_style(color="#315d9b")
    check("class-overrides-recipe", base, blue, {".notice": {"color": "#315d9b"}})
    check("typed-selector-style", base, blue, {".notice": style(color=token("accent"))})
    check("selector-source-order", base, blue, ".notice { color:#a0442e } .notice { color:#315d9b }")
    check("print-media-selector", base, blue,
          "@media print { .notice { color:#315d9b } } @media screen { .notice { color:#a0442e } }")
    check("selector-specificity", base, base, "p { color:#315d9b }")
    check("explicit-style-overrides-class", base.with_style(color="#245541"),
          base.with_style(color="#245541"), ".notice.notice { color:#315d9b }")
    check("important-class-overrides-normal-inline", base.with_style(color="#245541"), blue,
          ".notice { color:#315d9b !important }")
    important = base.with_style(css("color:#245541 !important"))
    check("important-inline-overrides-important-class", important, important,
          ".notice { color:#315d9b !important }")
    HiddenToken = styled(Paragraph, color=token("unused"))
    check("replaced-default-token-is-unused", HiddenToken("PAID IN FULL").with_style(color="#315d9b"),
          Paragraph("PAID IN FULL").with_style(color="#315d9b"))
    check("shorthand-and-instance-longhand", Card(base, class_name="panel", style=style(padding_left=pt(30))),
          Card(base, style=style(padding=pt(20), padding_left=pt(30))), ".panel { padding:20pt }")
    Spaced = styled(Flow, row_gap=pt(24))
    children = (Paragraph("PAID IN FULL"), Paragraph("Thank you for your work."))
    check("class-overrides-layout-default", Spaced(*children, class_name="notice"), Flow(*children, gap=pt(12)),
          ".notice { row-gap:12pt }")
    check("explicit-layout-overrides-class", Spaced(*children, class_name="notice", gap=pt(3)), Flow(*children, gap=pt(3)),
          ".notice { row-gap:12pt }")
    shared = StyleSheet({".notice": {"color": "#315d9b"}})
    captured = document(base, shared)
    before = captured.render_pdf(renderer)
    shared.add({".notice": {"color": "#a0442e"}})
    assert captured.render_pdf(renderer) == before
    check("conditional-styles", Paragraph("PAID IN FULL", style=style(False, None, color="#315d9b", margin=0)),
          Paragraph("PAID IN FULL").with_style(color="#315d9b", margin=0))

    # Page furniture remains usable while typed Page keeps actual PDF geometry.
    furniture = document(base, '@page { @bottom-center { content:"FULLBLEED"; font-family:Inter; font-size:8pt; margin-top:2pt } }')
    pdf = furniture.render_pdf(renderer)
    reader = PdfReader(BytesIO(pdf))
    assert "FULLBLEED" in "".join(reader.pages[0].extract_text().split())
    assert abs(float(reader.pages[0].mediabox.width) - 419.528) < .01
    (args.out / "page-furniture.pdf").write_bytes(pdf)
    report = {"ok": True, "checks": records, "shared_stylesheet_snapshotted": True,
              "page_furniture_and_typed_geometry": True}
    (args.out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rendered_controls": len(records), "page_furniture_and_typed_geometry": True}))


if __name__ == "__main__":
    main()
