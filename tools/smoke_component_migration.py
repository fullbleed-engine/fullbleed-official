# SPDX-License-Identifier: MIT
"""Verify incremental component migration against independently rendered PDFs."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui import Box as LegacyBox, Document as LegacyDocument, Paragraph as LegacyParagraph, StyleSheet
from fullbleed.ui.experimental import (
    Box, Card, Column, DataTable, Document, Flow, Fragment, Heading, Page, Paragraph,
    Renderer, Stack, Theme, component, mm, pt, style, styled, to_legacy, token,
)
from PIL import ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium


def compact(value):
    return "".join(value.split())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(fullbleed.__file__).parents, "Use python -I with the installed wheel"
    args.out.mkdir(parents=True, exist_ok=True)
    theme = Theme({"accent": "#315d9b", "muted": "#667580"},
                  text=style(font_family="Inter", font_size=pt(9), line_height=1.4, color="#263b46"))
    page = Page("A5", margin=mm(12))
    Label = styled(Paragraph, font_size=pt(8), font_weight=700, color=token("muted"))

    @component(parts=("label", "value"))
    def Field(label, value):
        return Stack(Label(label, part="label"), Paragraph(value, part="value"), gap=pt(4))

    field = Field("Account", "Maple & Finch").with_parts(label=style(color=token("accent")))
    table = DataTable([f"MIGRATED_ROW_{index:03d}" for index in range(75)], columns=[
        Column("Record", lambda row: row), Column("Amount", lambda row: "$123.45", align="right", width=mm(30)),
    ]).with_parts(header=style(background=token("accent"), color="#ffffff"))
    fixtures = {
        "field": Card(field, style=style(background="#edf3fa", border="1pt solid #315d9b")),
        "table": Flow(Heading("Migrated records", style=style(color=token("accent"))), table, gap=pt(12)),
        "fragment": Fragment(Heading("Reusable account", style=style(color=token("accent"))),
                             field, Paragraph("Literal text: <paid> & settled.")),
    }
    font = str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))
    options = {"font_files": [font], "document_timestamp": "2026-10-15T00:00:00Z"}
    legacy_styles = StyleSheet(page.to_css(), {
        "body": "margin: 0; box-sizing: border-box; " + theme.text.resolve(theme),
        "*": {"box_sizing": "border-box"},
        ".legacy-note": {"color": "#667580"},
    })
    cases = []
    for name, node in fixtures.items():
        title = f"Component migration: {name}"
        old_text = "This section still uses the existing stylesheet."
        children = to_legacy(node, theme=theme)
        @LegacyDocument(title=title, lang="en-US", page="A5", margin="12mm", styles=legacy_styles)
        def ExistingReport():
            return LegacyBox(LegacyParagraph(old_text, class_name="legacy-note", style={"margin": 0}), children)
        legacy = ExistingReport()
        reference = Document(Box(Paragraph(old_text, style=style(color="#667580")), node),
                             title=title, lang="en-US", page=page, theme=theme)
        engine = fullbleed.PdfEngine(**options, document_title=title, document_lang="en-US")
        html, css = legacy.to_html(), legacy.to_css()
        actual, glyphs = engine.render_pdf_with_glyph_report(html, css)
        assert not glyphs, (name, glyphs)
        assert actual == legacy.render_pdf(engine), (name, "repeat bytes")
        expected = reference.render_pdf(Renderer(**options))
        actual_reader, expected_reader = PdfReader(BytesIO(actual)), PdfReader(BytesIO(expected))
        assert len(actual_reader.pages) == len(expected_reader.pages), name
        assert actual_reader.metadata.title == title and actual_reader.trailer["/Root"]["/Lang"] == "en-US"
        actual_text = "\n".join(item.extract_text() for item in actual_reader.pages)
        expected_text = "\n".join(item.extract_text() for item in expected_reader.pages)
        assert compact(actual_text) == compact(expected_text), name
        if name == "table":
            assert len(actual_reader.pages) >= 3
            for index in range(75):
                assert compact(actual_text).count(f"MIGRATED_ROW_{index:03d}") == 1
        actual_doc, expected_doc = pdfium.PdfDocument(actual), pdfium.PdfDocument(expected)
        try:
            for index in range(len(actual_doc)):
                actual_image = actual_doc[index].render(scale=96 / 72).to_pil().convert("RGB")
                expected_image = expected_doc[index].render(scale=96 / 72).to_pil().convert("RGB")
                assert actual_image.size == expected_image.size
                assert ImageChops.difference(actual_image, expected_image).getbbox() is None, (name, index)
                actual_image.save(args.out / f"{name}-page-{index + 1}.png")
        finally:
            actual_doc.close()
            expected_doc.close()
        (args.out / f"{name}.pdf").write_bytes(actual)
        (args.out / f"{name}-reference.pdf").write_bytes(expected)
        (args.out / f"{name}.html").write_text(html, encoding="utf-8")
        (args.out / f"{name}.css").write_text(css, encoding="utf-8")
        cases.append({"name": name, "pages": len(actual_reader.pages), "converted_roots": len(children),
                      "pdf_sha256": sha256(actual).hexdigest(), "repeat_bytes_match": True,
                      "reference_pixels_match_at_96dpi": True, "reference_text_matches": True,
                      "metadata_matches": True, "missing_glyphs": 0})
    result = {"ok": True, "runtime": str(fullbleed.__file__), "cases": cases,
              "scope": "Three mixed old/new component fixtures; independent PDF pixels and text match equivalent experimental documents. This does not imply arbitrary stylesheet compatibility, automatic whole-document migration, or conformance."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases), "pages": sum(case["pages"] for case in cases)}))


if __name__ == "__main__":
    main()
