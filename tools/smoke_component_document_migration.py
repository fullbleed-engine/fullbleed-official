# SPDX-License-Identifier: MIT
"""Qualify the paired whole-document migration example with independent readers."""
from __future__ import annotations

import argparse
from hashlib import sha256
from html.parser import HTMLParser
from importlib.metadata import version
from io import BytesIO
import json
from pathlib import Path
import runpy

import fullbleed
from fullbleed import components, ui
from fullbleed.ui import experimental
from PIL import ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium


class HtmlStyles(HTMLParser):
    """Inspect the delivered HTML rather than only compiling another tree."""

    def __init__(self):
        super().__init__()
        self.blocks = []
        self.inline = []
        self.classes = set()
        self.in_style = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "style":
            self.in_style = True
            self.blocks.append("")
        if "style" in attrs:
            self.inline.append(attrs["style"])
        self.classes.update(attrs.get("class", "").split())

    def handle_endtag(self, tag):
        if tag == "style":
            self.in_style = False

    def handle_data(self, data):
        if self.in_style:
            self.blocks[-1] += data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(components.__file__).parents, "Use an installed wheel with python -I"
    assert components.Document is experimental.Document and components.Document is not ui.Document
    example = runpy.run_path(str(root / "examples/component_migration/render.py"))
    example["render"](args.out)
    options = example["renderer_options"]()
    cases = []
    for report in example["REPORTS"]:
        old_path = args.out / f"{report.name}-legacy.pdf"
        new_path = args.out / f"{report.name}-components.pdf"
        old, new = old_path.read_bytes(), new_path.read_bytes()
        old_reader, new_reader = PdfReader(BytesIO(old)), PdfReader(BytesIO(new))
        assert len(old_reader.pages) == len(new_reader.pages) >= 2, report.name
        for reader in (old_reader, new_reader):
            assert reader.metadata.title == report.title
            assert reader.trailer["/Root"]["/Lang"] == report.lang
            text = "\n".join(page.extract_text() for page in reader.pages)
            for i in range(1, 41):
                assert text.count(f"ENTRY_{i:03d}") == 1, (report.name, i)
        assert [page.extract_text() for page in old_reader.pages] == [page.extract_text() for page in new_reader.pages]
        expected_points = (792, 612) if report.landscape else (148 * 72 / 25.4, 210 * 72 / 25.4)
        media_boxes = []
        for old_page, new_page in zip(old_reader.pages, new_reader.pages):
            box = [float(value) for value in new_page.mediabox]
            assert box == [float(value) for value in old_page.mediabox]
            assert all(abs(actual - expected) < .02 for actual, expected in zip(box[2:], expected_points))
            media_boxes.append(box)
        migrated = example["migrated_document"](report)
        renderer = components.Renderer(**options)
        assert renderer.engine_for(migrated).document_timestamp == example["TIMESTAMP"]
        assert new == migrated.render_pdf(renderer), (report.name, "repeat bytes")
        legacy = example["legacy_document"](report)
        engine = fullbleed.PdfEngine(**options, document_title=report.title, document_lang=report.lang)
        assert engine.document_timestamp == example["TIMESTAMP"]
        assert old == legacy.render_pdf(engine), (report.name, "legacy repeat bytes")
        source = migrated.compile(target="pdf")
        for html, css in ((legacy.to_html(), legacy.to_css()), (source.html, source.css)):
            _, glyphs = engine.render_pdf_with_glyph_report(html, css)
            assert not glyphs, (report.name, glyphs)
        styled = (args.out / f"{report.name}-styled.html").read_text(encoding="utf-8")
        unstyled = (args.out / f"{report.name}-unstyled.html").read_text(encoding="utf-8")
        styled_check, unstyled_check = HtmlStyles(), HtmlStyles()
        styled_check.feed(styled)
        unstyled_check.feed(unstyled)
        assert styled_check.blocks == [migrated.to_css()], (report.name, "saved styled HTML must embed the complete stylesheet")
        assert "muted" in styled_check.classes and ".muted" in styled_check.blocks[0]
        assert not unstyled_check.blocks and not unstyled_check.inline
        assert "muted" in unstyled_check.classes
        assert styled == migrated.to_html(include_styles=True)
        assert unstyled == migrated.to_html(include_styles=False)
        assert "@page" in source.css and "margin: " in source.css
        assert "@page" not in migrated.styles, "Page geometry must be owned by Page"
        old_doc, new_doc = pdfium.PdfDocument(old), pdfium.PdfDocument(new)
        try:
            for index in range(len(old_doc)):
                before = old_doc[index].render(scale=96 / 72).to_pil().convert("RGB")
                after = new_doc[index].render(scale=96 / 72).to_pil().convert("RGB")
                before.save(args.out / f"{report.name}-legacy-page-{index + 1}.png")
                after.save(args.out / f"{report.name}-components-page-{index + 1}.png")
                assert before.size == after.size
                assert ImageChops.difference(before, after).getbbox() is None, (report.name, index)
        finally:
            old_doc.close()
            new_doc.close()
        cases.append({"name": report.name, "pages": len(new_reader.pages), "media_boxes": media_boxes,
                      "margin_mm_clockwise": report.margins, "title": report.title, "lang": report.lang,
                      "legacy_pdf_sha256": sha256(old).hexdigest(), "migrated_pdf_sha256": sha256(new).hexdigest(),
                      "text_per_page_matches": True, "pixels_match_at_96dpi": True,
                      "metadata_matches": True, "resolved_engine_timestamp": engine.document_timestamp,
                      "repeat_bytes_match_per_path": True, "missing_glyphs": 0,
                      "styled_export_embeds_complete_stylesheet": True,
                      "unstyled_export_retains_literal_classes": True})
    result = {"ok": True, "version": version("fullbleed"), "runtime": str(components.__file__), "cases": cases,
              "scope": "Two explicit whole-document migrations, including portrait/landscape geometry, asymmetric margins, component styles, selector CSS, title/language, fonts and fixed timestamps. Old and new paths match in independent PDF text and pixels. Not an automatic converter or arbitrary stylesheet compatibility claim."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases), "pages_per_path": sum(case["pages"] for case in cases)}))


if __name__ == "__main__":
    main()
