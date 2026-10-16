# SPDX-License-Identifier: MIT
"""Verify theme derivation/discovery against installed component PDFs."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui.experimental import (
    Box, Document, Flow, Heading, Page, Paragraph, Renderer, Theme, component,
    mm, pt, style, styled, token,
)
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


@component(parts=("label",), optional_parts=("note",))
def Notice(text, *, note=None):
    Default = styled(Box, color=token("overridden"))
    return Default(
        Heading(text, level=1, part="label", style=style(font_size=pt(25))),
        Paragraph(note, part="note") if note else None,
    ).with_style(background=token("accent"), color=token("paper"), padding=token("padding"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-15T00:00:00Z")
    base = Theme({"accent": "#315d9b", "paper": "#ffffff", "ink": "#263b46", "padding": pt(14), "size": pt(10), "unused": "green"},
                 text=style(font_family="Inter", font_size=token("size"), color=token("ink"), line_height=1.4))
    shared = Notice("One kit. Your brand.").with_parts(note=style(color=token("absent")))
    assert shared.required_tokens == ("accent", "padding", "paper")
    content = Flow(shared, Paragraph("Rebrand a shared component without rewriting its typography or changing the original document."), gap=pt(12))
    original = Document(content, title="Theme derivation", page=Page("A5", margin=mm(14)), theme=base)
    original_pdf = original.render_pdf(renderer)
    cases = []
    variants = [
        ("base", base, dict(base.values)),
        ("warm", base.with_values(accent="#9a462e"), {**dict(base.values), "accent": "#9a462e"}),
        ("large", base.with_values({"accent": "#245541", "padding": pt(24)}, size=pt(14)),
         {**dict(base.values), "accent": "#245541", "padding": pt(24), "size": pt(14)}),
    ]
    for name, theme, expected_values in variants:
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        document = Document(content, title=original.title, page=original.page, theme=theme)
        assert document.required_tokens == ("accent", "ink", "padding", "paper", "size")
        assert theme.text == base.text
        pdf = document.render_pdf(renderer)
        reference = Document(content, title=original.title, page=original.page, theme=Theme(expected_values, text=base.text))
        assert pdf == reference.render_pdf(renderer)
        plan = document.compile()
        repeated, glyphs = renderer.engine_for(document).render_pdf_with_glyph_report(plan.html, plan.css)
        assert repeated == pdf and not glyphs
        reader = PdfReader(BytesIO(pdf))
        assert len(reader.pages) == 1
        assert "One kit. Your brand." in reader.pages[0].extract_text()
        (folder / "document.pdf").write_bytes(pdf)
        document.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        with closing(pdfium.PdfDocument(pdf)) as independent, closing(independent[0]) as page, closing(page.render(scale=1)) as bitmap:
            image = bitmap.to_pil().convert("RGB")
            image.save(folder / "independent-page-1.png")
            colors = {rgb: count for count, rgb in image.getcolors(1000000)}
            assert colors[tuple(bytes.fromhex(expected_values["accent"].removeprefix("#")))] > 1000
        cases.append(dict(name=name, pages=1, pdf_sha256=sha256(pdf).hexdigest(), required_tokens=list(document.required_tokens),
                          matches_explicit_constructor=True, glyphs=0, accent=expected_values["accent"]))
    assert original.render_pdf(renderer) == original_pdf
    assert len({case["pdf_sha256"] for case in cases}) == 3
    result = {"ok": True, "installed_module": fullbleed.__file__, "cases": cases, "original_unchanged": True}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases), "pages": 3, "original_unchanged": True}))


if __name__ == "__main__":
    main()
