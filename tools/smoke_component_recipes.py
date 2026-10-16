# SPDX-License-Identifier: MIT
"""Render the experimental authoring workflow from an installed candidate wheel."""
from __future__ import annotations

import argparse
from dataclasses import asdict
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

import fullbleed
from fullbleed.ui.experimental import (
    Column, DataTable, Document, Flow, Page, Paragraph, Renderer, Theme,
    mm, pt, style,
)
from PIL import Image
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


def compact(text):
    return "".join(text.casefold().split())


def oversized_document():
    markers = [f"ROW_SEGMENT_{index:03d}" for index in range(1, 91)]
    table = DataTable([markers], columns=[Column("Segments", lambda row: Flow([Paragraph(value) for value in row], gap=pt(3)))])
    return Document(table, page=Page("A5", margin=mm(12)),
                    theme=Theme(text=style(font_family="Inter", font_size=pt(9), line_height=1.45)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "target/component-recipes")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python/fullbleed" not in Path(fullbleed.__file__).parents
    sys.path.insert(0, str(ROOT / "examples/component_recipes"))
    try:
        from documents import Invoice, OperationsReport
        from kit import Field, studio_theme
    finally:
        sys.path.pop(0)
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    renderer = Renderer(font_files=[str(font)], document_timestamp="2026-10-15T00:00:00Z")
    data = json.loads((ROOT / "examples/components/data.json").read_text(encoding="utf-8"))
    cases = []

    def check(name, document, present, *, size, pages=None, accent=None, repeated_header=None):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        plan = document.compile()
        engine = renderer.engine_for(document)
        pdf, glyphs = engine.render_pdf_with_glyph_report(plan.html, plan.css)
        assert not glyphs, (name, glyphs)
        assert pdf == document.render_pdf(renderer), (name, "deterministic rendering")
        reader = PdfReader(BytesIO(pdf))
        assert reader.metadata.title == document.title
        assert reader.trailer["/Root"]["/Lang"] == document.lang
        if pages is not None:
            assert len(reader.pages) == pages, (name, len(reader.pages))
        text = " ".join(page.extract_text() for page in reader.pages)
        for expected in present:
            assert compact(expected) in compact(text), (name, "missing", expected)
        if repeated_header is not None:
            matches = [compact("ProjectHoursRateAmount") in compact(page.extract_text()) for page in reader.pages]
            expected_headers = [True] * len(reader.pages) if repeated_header else [True] + [False] * (len(reader.pages) - 1)
            assert matches == expected_headers, (name, matches)
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "text.txt").write_text(text, encoding="utf-8")
        document.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        (folder / "sources.json").write_text(json.dumps([asdict(source) for source in plan.sources], indent=2) + "\n", encoding="utf-8")
        previews = document.render_image_pages(renderer, dpi=96)
        assert len(previews) == len(reader.pages)
        for index, preview in enumerate(previews, 1):
            (folder / f"page-{index}.png").write_bytes(preview)
        if accent:
            color = tuple(bytes.fromhex(accent.removeprefix("#")))
            counts = {rgb: count for count, rgb in Image.open(BytesIO(previews[0])).convert("RGB").getcolors(1000000)}
            assert counts[color] > 1000, (name, "theme color", counts[color])
        independent = pdfium.PdfDocument(pdf)
        character_count = 0
        for page_index in range(len(independent)):
            page = independent[page_index]
            try:
                width, height = page.get_size()
                assert abs(width - size[0]) < .1 and abs(height - size[1]) < .1, (name, width, height)
                text_page = page.get_textpage()
                try:
                    for index in range(text_page.count_chars()):
                        if not text_page.get_text_range(index, 1).strip():
                            continue
                        left, bottom, right, top = text_page.get_charbox(index)
                        assert left >= -1 and bottom >= -1 and right <= width + 1 and top <= height + 1, (name, page_index, index, (left, bottom, right, top))
                        character_count += 1
                finally:
                    text_page.close()
                if page_index == 0:
                    bitmap = page.render(scale=1)
                    bitmap.to_pil().save(folder / "independent-page-1.png")
                    bitmap.close()
            finally:
                page.close()
        independent.close()
        cases.append({"name": name, "pages": len(reader.pages), "pdf_sha256": sha256(pdf).hexdigest(),
                      "missing_glyphs": 0, "checked_characters": character_count, "expected_text": len(present),
                      "page_size_pt": size, "metadata_matches": True, "repeat_bytes_match": True,
                      "theme_color": accent, "sources": len(plan.sources)})
        return reader, text

    a4, letter, a5 = (595.276, 841.89), (612, 792), (419.528, 595.276)
    check("invoice", Invoice(data), [data["customer"], "$8,100.00"], size=a4, pages=1, accent="#315d9b")
    check("brand-change", Invoice(data, accent="#9a462e"), [data["customer"], "$8,100.00"], size=a4, pages=1, accent="#9a462e")
    paid = {**data, "paid": True}
    check("paid", Invoice(paid), ["PAID IN FULL", "TOTAL PAID", "$8,100.00"], size=a4, pages=1)
    check("empty", Invoice({**data, "items": []}), ["No billable items.", "$0.00"], size=a4, pages=1)
    escaped = {**data, "customer": '<b>A & B</b>'}
    check("escaped", Invoice(escaped), [escaped["customer"]], size=a4, pages=1)
    credit = {**data, "items": [{**data["items"][0], "quantity": 1, "rate_cents": -150}]}
    check("credit", Invoice(credit), ["-$1.50"], size=a4, pages=1)
    long = {**data, "items": [{**data["items"][index % 3], "name": f"Deliverable {index:03d}"} for index in range(1, 61)]}
    _, text = check("report", OperationsReport(long), [item["name"] for item in long["items"]], size=letter, repeated_header=True)
    for item in long["items"]:
        assert compact(text).count(compact(item["name"])) == 1, ("duplicated row", item["name"])

    shared = Field("Portable component", "PORTABLE_VALUE")
    check("portable-a5", Document(shared, page=Page("A5", margin=mm(12)), theme=studio_theme()), ["PORTABLE_VALUE"], size=a5, pages=1)
    check("portable-custom-page", Document(shared, page=Page((mm(100), mm(150)), margin=mm(10)), theme=studio_theme()),
          ["PORTABLE_VALUE"], size=(283.465, 425.197), pages=1)

    # Keep the unsupported case explicit. Exercise it in a bounded child process
    # so a regression of the repeated-header progress guard cannot hang this gate.
    oversized = oversized_document()
    folder = args.out / "oversized-row"
    folder.mkdir(exist_ok=True)
    plan = oversized.compile()
    (folder / "document.html").write_text(plan.html, encoding="utf-8")
    (folder / "document.css").write_text(plan.css, encoding="utf-8")
    child = """import json, runpy, sys
from fullbleed.ui.experimental import AuthoringError, Renderer
document = runpy.run_path(sys.argv[1])['oversized_document']()
renderer = Renderer(font_files=[sys.argv[2]], document_timestamp='2026-10-15T00:00:00Z')
try:
    document.render_pdf(renderer)
except AuthoringError as error:
    print(json.dumps(error.to_dict()))
else:
    raise AssertionError('Oversized rows now render; replace the known-gap check with content-preservation evidence')
"""
    started = time.monotonic()
    failure = subprocess.run([sys.executable, "-I", "-c", child, str(Path(__file__).resolve()), str(font)],
                             capture_output=True, text=True, encoding="utf-8", timeout=20)
    assert failure.returncode == 0, failure.stderr
    error = json.loads(failure.stdout)
    assert error["code"] == "layout.unsplittable", error
    unsupported = {"name": "oversized-row", "behavior": "bounded component renderer rejection", "error": error,
                   "elapsed_seconds": time.monotonic() - started, "content_preserving_continuation": False}
    (folder / "known-gap.json").write_text(json.dumps(unsupported, indent=2) + "\n", encoding="utf-8")

    # A single extra column declares both its header and all corresponding cells.
    columns = [Column("Project", lambda row: row), Column("Hours", lambda row: "2"),
               Column("Rate", lambda row: "$5"), Column("Amount", lambda row: "$10")]
    table = DataTable((f"ITEM_{index:03d}" for index in range(70)), columns=columns, repeat_header=False)
    check("headers-disabled", Document(table, page=Page("A5", margin=mm(12)), theme=studio_theme()),
          ["ITEM_000", "ITEM_069", "$10"], size=a5, repeated_header=False)

    # One reusable override must work before and after records arrive. The unused
    # optional part has no paint, while the visible part keeps its supplied style.
    for name, rows, expected in (("optional-parts-empty", [], "NO_ROWS"),
                                 ("optional-parts-filled", ["ONE_ROW"], "ONE_ROW")):
        table = DataTable(rows, columns=[Column("Item", str)], empty="NO_ROWS").with_parts(
            cell=style(background="#dce8f5"), empty=style(background="#dce8f5"),
            caption=style(color="#315d9b"),
        )
        assert table.parts == DataTable.parts
        check(name, Document(table, theme=studio_theme()), [expected], size=a4, pages=1, accent="#dce8f5")

    snippets = re.findall(r"```python\n(.*?)```", (ROOT / "examples/component_recipes/README.md").read_text(encoding="utf-8"), re.S)
    previous = Path.cwd()
    namespace = {}
    snippet_dir = (args.out / "readme").resolve()
    snippet_dir.mkdir(exist_ok=True)
    try:
        os.chdir(snippet_dir)
        for index, snippet in enumerate(snippets, 1):
            exec(compile(snippet, f"README block {index}", "exec"), namespace)
    finally:
        os.chdir(previous)
    assert PdfReader(snippet_dir / "customer.pdf").metadata.title == "Customer details"

    result = {"ok": True, "version": metadata.version("fullbleed"), "runtime": str(fullbleed.__file__),
              "cases": cases, "readme_blocks": len(snippets), "ready_for_stabilization": False, "unsupported_cases": [unsupported],
              "scope": "Experimental authoring, metadata, geometry, ordinary table continuation, data edits, theme colors, source maps, glyphs and deterministic fixture bytes. Oversized rows are explicitly unsupported. Not a conformance or performance claim."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases), "pages": sum(case["pages"] for case in cases)}))


if __name__ == "__main__":
    main()
