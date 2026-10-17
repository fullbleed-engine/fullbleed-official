# SPDX-License-Identifier: MIT
"""Retain installed-wheel Bootstrap evidence (test-only pypdf, PDFium and Pillow)."""
from __future__ import annotations

import argparse
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import runpy
import sys

import fullbleed.ui.experimental as ui
from fullbleed.ui.experimental import bootstrap as bs
from pypdf import PdfReader
import pypdfium2 as pdfium


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(ui.__file__).parents, "Use an installed wheel with python -I"
    args.out.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, str(root / "examples/component_bootstrap"))
    example = runpy.run_path(str(root / "examples/component_bootstrap/main.py"))
    sys.argv = ["main.py", "--out", str(args.out), "--catalog-pdfs"]
    example["main"]()
    trace = json.loads((args.out / "catalog.pagination.json").read_text())
    assert set(bs.COMPONENT_FAMILIES) <= trace.keys()
    assert all("error" not in value and value["pages"] == 1 for value in trace.values()), trace
    pdfs = {}
    for path in sorted(args.out.glob("*.pdf")):
        reader = PdfReader(path)
        assert reader.pages
        text = "\n".join(page.extract_text() for page in reader.pages)
        assert text.strip(), path
        pdfs[path.stem] = {"pages": len(reader.pages), "text": text,
                           "sha256": sha256(path.read_bytes()).hexdigest()}
    for value in ("Northstar Studio", "Company", "Standard stock", "Handle with care", "50 / 100"):
        assert value in pdfs["catalog-forms"]["text"], value
    assert "Premium stock" not in pdfs["catalog-forms"]["text"]
    assert "Inactive record history" not in pdfs["catalog-navs-tabs"]["text"]
    assert "Inactive collection" not in pdfs["catalog-carousel"]["text"]
    numbered = "".join(pdfs["catalog-list-group"]["text"].split())
    assert "1.Firstcheckpoint" in numbered and "2.Finalcheckpoint" in numbered, numbered
    for value in ("408", "$8,640", "98%", "Studio notebooks", "$2,880.00", "75%"):
        assert value in pdfs["report"]["text"], value
    font = __import__("importlib.resources", fromlist=["files"]).files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    renderer = ui.Renderer(font_files=[str(font)], document_timestamp="2026-04-30T00:00:00Z")
    again = example["report"]().render_pdf(renderer)
    assert sha256(again).hexdigest() == pdfs["report"]["sha256"], "Repeated report PDF bytes changed"
    glyph_checks = {}
    documents = {"report": example["report"](), **{
        name: ui.Document(node, theme=ui.Theme(text=ui.style(font_family="Inter", font_size=ui.pt(10))))
        for name, node in example["samples"]().items()
    }}
    for name, document in documents.items():
        plan = document.compile(target="pdf")
        _, missing = renderer.engine_for(document).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not missing, (name, missing)
        glyph_checks[name] = missing
    long_table = ui.Document(
        bs.DataTable(list(range(1, 81)), columns=[bs.Column("Record", lambda value: f"RECORD{value:03d}"),
                                                bs.Column("Quantity", lambda value: value, align="right")], striped=True),
        page=ui.Page("A5", margin=ui.mm(12)), theme=ui.Theme(text=ui.style(font_family="Inter", font_size=ui.pt(10))),
    )
    long_pdf = long_table.render_pdf(renderer)
    (args.out / "long-table.pdf").write_bytes(long_pdf)
    long_reader = PdfReader(BytesIO(long_pdf))
    long_text = "\n".join(page.extract_text() for page in long_reader.pages)
    assert len(long_reader.pages) > 1
    assert all(long_text.count(f"RECORD{value:03d}") == 1 for value in range(1, 81))
    assert all("Record" in page.extract_text() and "Quantity" in page.extract_text() for page in long_reader.pages)
    raster_checks = {}
    for name in ("report", "catalog-forms", "catalog-buttons", "catalog-pagination"):
        pdf = pdfium.PdfDocument(args.out / (name + ".pdf"))
        page = pdf[0]
        bitmap = page.render(scale=1.4)
        image = bitmap.to_pil().convert("RGB")
        image.save(args.out / (name + ".pdfium.png"))
        if name == "catalog-pagination":
            textpage = page.get_textpage()
            text = textpage.get_text_range()
            colors = {}
            for label, expected in (("1", (13, 110, 253)), ("2", (255, 255, 255))):
                left, bottom, right, top = textpage.get_charbox(text.index(label))
                point = (round((left + right) / 2 * 1.4), round((page.get_height() - bottom + 2) * 1.4))
                actual = image.getpixel(point)
                assert actual == expected, (label, actual, expected)
                colors[label] = actual
            raster_checks["pagination_state_fills"] = colors
            textpage.close()
        bitmap.close()
        page.close()
        pdf.close()
    result = {"ok": True, "runtime": ui.__file__, "bootstrap": bs.VERSION,
              "css_sha256": sha256(bs.stylesheet().encode()).hexdigest(),
              "component_families": sorted(bs.COMPONENT_FAMILIES), "specimens": len(trace), "pdfs": pdfs,
              "repeated_report_bytes_equal": True,
              "missing_glyphs": glyph_checks,
              "long_table": {"rows": 80, "pages": len(long_reader.pages), "each_row_once": True, "headers_on_each_page": True},
              "independent_raster_checks": raster_checks,
              "scope": "27 authored static specimens plus one report. Content/state assertions and native pagination checks; not complete Bootstrap CSS or accessibility conformance."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({"ok": True, "specimens": len(trace), "pdfs": len(pdfs)}))


if __name__ == "__main__":
    main()
