# SPDX-License-Identifier: MIT
"""Explicit whole-document migration from fullbleed.ui to fullbleed.components.

Both authoring paths generate the same small account report. No HTML parser or
implicit conversion connects them. Run with an installed candidate wheel.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from importlib.resources import files
from pathlib import Path

import fullbleed
from fullbleed import ui
from fullbleed.components import (
    Box, Document, Heading, Page, Paragraph, Renderer, Theme, pt, mm, style, styled, token,
)


@dataclass(frozen=True)
class Report:
    name: str
    paper: str
    landscape: bool
    # Clockwise: top, right, bottom, left. Units are millimetres.
    margins: tuple[int, int, int, int]
    title: str
    lang: str = "en-US"


REPORTS = (
    Report("a5", "A5", False, (14, 11, 17, 13), "Maple & Finch | Account report"),
    Report("letter-landscape", "LETTER", True, (15, 20, 18, 12), "Maple & Finch | Landscape report"),
)
TIMESTAMP = "2026-10-21T00:00:00Z"
INK = "#183e36"
MUTED = "#50665f"
ROWS = tuple(f"ENTRY_{i:03d}  -  Document services  -  $125.00" for i in range(1, 41))


def legacy_document(report: Report) -> ui.DocumentArtifact:
    """Old path: stylesheet owns geometry; PdfEngine receives metadata below."""
    size = report.paper + (" landscape" if report.landscape else "")
    margins = " ".join(f"{value}mm" for value in report.margins)
    styles = ui.StyleSheet(
        f"@page {{ size: {size}; margin: {margins}; }}",
        {
            "*": {"box_sizing": "border-box"},
            "body": {"margin": 0, "font_family": "Inter", "font_size": "10pt",
                     "line_height": 1.4, "color": INK},
            "p": {"margin": 0},
            ".report-title": {"margin": 0, "font_size": "22pt", "line_height": 1.15,
                              "font_weight": 700, "color": INK, "break_after": "avoid"},
            ".summary": {"padding": "12pt", "margin_top": "14pt", "margin_bottom": "14pt",
                         "background": "#eef3ed", "border_left": f"3pt solid {INK}"},
            ".entry": {"padding_top": "6pt", "padding_bottom": "6pt",
                       "border_bottom": "0.5pt solid #d9e2da", "break_inside": "avoid"},
            ".muted": {"color": MUTED},
        },
    )

    @ui.Document(page=report.paper, margin=margins, title=report.title, lang=report.lang,
                 bootstrap=False, include_component_styles=False, styles=styles)
    def AccountReport():
        return ui.Box(
            ui.Heading(report.title, level=1, class_name="report-title"),
            ui.Paragraph("Statement period: October 2026", class_name="muted"),
            ui.Box(ui.Paragraph("Maple & Finch"),
                   ui.Paragraph("40 document projects. Total: $5,000.00."), class_name="summary"),
            [ui.Paragraph(row, class_name="entry") for row in ROWS],
            ui.Paragraph("Thank you for your partnership.", class_name="muted"),
        )

    return AccountReport()


def migrated_document(report: Report) -> Document:
    """New path: Document owns geometry and metadata, components own defaults."""
    Title = styled(Heading, font_size=pt(22), line_height=1.15, color=token("ink"))
    Summary = styled(Box, padding=pt(12), margin_top=pt(14), margin_bottom=pt(14),
                     background="#eef3ed", border_left=f"3pt solid {INK}")
    Entry = styled(Paragraph, padding_top=pt(6), padding_bottom=pt(6),
                   border_bottom="0.5pt solid #d9e2da", break_inside="avoid")
    return Document(
        Box(
            Title(report.title, level=1),
            Paragraph("Statement period: October 2026", class_name="muted"),
            Summary(Paragraph("Maple & Finch"),
                    Paragraph("40 document projects. Total: $5,000.00.")),
            [Entry(row) for row in ROWS],
            Paragraph("Thank you for your partnership.", class_name="muted"),
        ),
        title=report.title,
        lang=report.lang,
        page=Page(report.paper, landscape=report.landscape,
                  margin=tuple(mm(value) for value in report.margins)),
        theme=Theme({"ink": INK, "muted": MUTED},
                    text=style(font_family="Inter", font_size=pt(10), line_height=1.4, color=token("ink"))),
        # class_name is a literal selector hook. This explicit rule supplies its appearance.
        styles={".muted": style(color=token("muted"))},
    )


def renderer_options(font: Path | None = None) -> dict:
    explicit_font = font or Path(str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")))
    return {"font_files": [str(explicit_font)], "document_timestamp": TIMESTAMP}


def render(out: Path, *, font: Path | None = None) -> None:
    out.mkdir(parents=True, exist_ok=True)
    options = renderer_options(font)
    for report in REPORTS:
        legacy = legacy_document(report)
        migrated = migrated_document(report)
        # Before: keep page CSS, engine metadata and fonts consistent yourself.
        engine = fullbleed.PdfEngine(**options, document_title=report.title, document_lang=report.lang)
        (out / f"{report.name}-legacy.pdf").write_bytes(legacy.render_pdf(engine))
        (out / f"{report.name}-legacy.html").write_text(legacy.to_html(), encoding="utf-8")
        (out / f"{report.name}-legacy.css").write_text(legacy.to_css(), encoding="utf-8")
        # After: Renderer accepts resource/options; Document supplies its own metadata and Page.
        (out / f"{report.name}-components.pdf").write_bytes(migrated.render_pdf(Renderer(**options)))
        (out / f"{report.name}-styled.html").write_text(
            migrated.to_html(include_styles=True), encoding="utf-8")
        (out / f"{report.name}-unstyled.html").write_text(
            migrated.to_html(include_styles=False), encoding="utf-8")
        source = migrated.compile(target="pdf")
        (out / f"{report.name}-components.html").write_text(source.html, encoding="utf-8")
        (out / f"{report.name}-components.css").write_text(source.css, encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path(__file__).parent / "output")
    parser.add_argument("--font", type=Path, help="Explicit Inter font, if not using the bundled asset")
    args = parser.parse_args()
    render(args.out, font=args.font)
    print(f"Wrote paired reports to {args.out.resolve()}")
