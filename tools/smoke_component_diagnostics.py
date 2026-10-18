# SPDX-License-Identifier: MIT
"""Check native component failures and repaired PDFs from an installed wheel."""
from __future__ import annotations

import argparse
from contextlib import closing
from dataclasses import asdict
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path
import subprocess
import sys

import fullbleed
from fullbleed.ui.experimental import (
    Box, Column, DataTable, Document, Flow, LayoutError, Page, Paragraph, Renderer,
    Theme, component, pt, style,
)
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
PAPER = Page((pt(240), pt(180)), margin=pt(12))
THEME = Theme(text=style(font_family="Inter", font_size=pt(10), line_height=1.4))
MARKERS = [f"CONTENT_{index:03d}" for index in range(30)]


@component
def WorkTable(*, align="top", multiple=False, header_height=None):
    def content(long):
        return Flow([Paragraph(marker) for marker in MARKERS], gap=0) if long else Paragraph("FIRST_ROW")
    columns = [Column("Description", content)]
    if multiple:
        columns.append(Column("Amount", lambda long: "SHORT_CELL" if long else "FIRST_AMOUNT"))
    table = DataTable([False, True], columns=columns).with_parts(cell=style(vertical_align=align))
    return table.with_parts(header=style(height=header_height)) if header_height is not None else table


@component
def ClippedCard():
    return Box(Paragraph("CARD_CONTENT")).with_style(height=pt(500), overflow="hidden")


@component
def PaddedCard():
    return Box(Paragraph("CARD_CONTENT")).with_style(padding_top=pt(300))


def document(node):
    return Document(node, title="Diagnostic examples", page=PAPER, theme=THEME)


def failure_cases():
    return {
        "nested-table-cell": (document(Box(Box(WorkTable(align="bottom")))), "table_cell_alignment",
                              [("WorkTable", "DataTable", "row 2", "column 1", "Description", "cell")], 2),
        "multiple-table-cells": (document(WorkTable(align="bottom", multiple=True)), "table_cell_alignment",
                                 [("WorkTable", "DataTable", "row 2", f"column {i}", label, "cell")
                                  for i, label in enumerate(("Description", "Amount"), 1)], 2),
        "completed-companion": (document(DataTable([0], columns=[
            Column("Blocked", lambda _: ClippedCard()), Column("Complete", lambda _: "FINISHED_CELL")
        ])), "table_cell_content", [("DataTable", "row 1", "column 1", "Blocked", "cell")], 2),
        "duplicate-column-labels": (document(DataTable([0], columns=[
            Column("Value", lambda _: "FINISHED_CELL"), Column("Value", lambda _: ClippedCard())
        ])), "table_cell_content", [("DataTable", "row 1", "column 2", "Value", "cell")], 2),
        "clipping-owner": (document(Flow(ClippedCard())), "clipped_container", [("ClippedCard", "div")], 1),
        "padding-owner": (document(Box(PaddedCard())), "container_decoration_height", [("PaddedCard", "div")], 1),
        "table-header-space": (document(WorkTable(header_height=pt(300))), "table_fixed_height",
                               [("WorkTable", "DataTable", "table")], 1),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--case", choices=tuple(failure_cases()), help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-15T00:00:00Z")
    if args.case:
        doc, reason, paths, page = failure_cases()[args.case]
        plan = doc.compile()
        doc.emit_artifacts(html_path=args.out / "document.html", css_path=args.out / "document.css")
        (args.out / "sources.json").write_text(json.dumps([asdict(source) for source in plan.sources], indent=2) + "\n")
        try:
            doc.render_pdf(renderer)
        except LayoutError as error:
            assert error.reason == reason, error.to_dict()
            assert error.page == page, error.to_dict()
            assert [source.path for source in error.failure_sources] == paths, error.to_dict()
            assert all(source.file and source.line for source in error.failure_sources), error.to_dict()
            assert error.sources == (plan.sources[0],), error.to_dict()
            expected_path = (doc.title, f"page {page}", *(paths[0] if len(paths) == 1 else ()))
            assert error.path == expected_path, error.to_dict()
            result = {"name": args.case, "error": error.to_dict(), "runtime": str(fullbleed.__file__)}
            (args.out / "error.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            print(json.dumps(result))
            return
        raise AssertionError("An unsplittable document must be rejected")

    rejected = []
    for name in failure_cases():
        run = subprocess.run([sys.executable, "-I", str(Path(__file__).resolve()), "--case", name,
                              "--out", str(args.out / name)], capture_output=True, text=True,
                             encoding="utf-8", timeout=20)
        assert run.returncode == 0, (name, run.stdout, run.stderr)
        rejected.append(json.loads(run.stdout))

    repaired = []
    for name, doc, markers in (
        ("top-aligned-table", document(Box(Box(WorkTable()))), ["FIRST_ROW", *MARKERS]),
        ("baseline-aligned-table", document(Box(Box(WorkTable(align="baseline")))), ["FIRST_ROW", *MARKERS]),
        ("card-fits", document(Flow(ClippedCard().with_style(height="auto", overflow="visible"))), ["CARD_CONTENT"]),
        ("padding-fits", document(Box(PaddedCard().with_style(padding_top=pt(6)))), ["CARD_CONTENT"]),
    ):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        plan = doc.compile()
        pdf = doc.render_pdf(renderer)
        assert pdf == doc.render_pdf(renderer), (name, "repeat bytes")
        _, glyphs = renderer.engine_for(doc).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not glyphs, (name, glyphs)
        reader = PdfReader(BytesIO(pdf))
        text = "".join("".join(page.extract_text().split()) for page in reader.pages)
        assert all(text.count(marker) == 1 for marker in markers), (name, text)
        (folder / "document.pdf").write_bytes(pdf)
        doc.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        with closing(pdfium.PdfDocument(pdf)) as rendered:
            for index in range(len(rendered)):
                with closing(rendered[index]) as page, closing(page.render(scale=1.5)) as bitmap, closing(page.get_textpage()) as text_page:
                    bitmap.to_pil().save(folder / f"independent-page-{index + 1}.png")
                    width, height = page.get_size()
                    for char in range(text_page.count_chars()):
                        if text_page.get_text_range(char, 1).strip():
                            left, bottom, right, top = text_page.get_charbox(char)
                            assert -1 <= left <= right <= width + 1 and -1 <= bottom <= top <= height + 1, (name, index, char)
        repaired.append({"name": name, "pages": len(reader.pages), "pdf_sha256": sha256(pdf).hexdigest(),
                         "repeat_bytes_equal": True, "missing_glyphs": 0, "markers": len(markers)})
    result = {"ok": True, "rejected": rejected, "repaired": repaired,
              "scope": "Native split rejections identify actual cell constraints or container owners through block and authored wrappers. Original fragment scopes remain available. Not all layout wrappers report descendant causes; unknown failures retain their available scope. Repairs check complete text, glyphs, page bounds and repeated PDF bytes."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rejected": len(rejected), "repaired": len(repaired), "pages": sum(case["pages"] for case in repaired)}))


if __name__ == "__main__":
    main()
