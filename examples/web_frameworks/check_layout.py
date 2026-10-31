# SPDX-License-Identifier: MIT
"""Check content, pagination and visible text bounds in final invoice PDFs."""
from decimal import Decimal
from pathlib import Path
import hashlib
import json

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium

from invoice import load_invoice, render_invoice


def compact(text: str) -> str:
    return "".join(text.split())


def inspect_invoice(path: Path, data: dict) -> dict:
    """Text extraction includes off-page text, so also inspect every glyph box."""
    inspection = dict(fullbleed.inspect_pdf(str(path)))
    assert not inspection["warnings"], (path, inspection["warnings"])
    assert not inspection["composition"]["issues"], (path, inspection["composition"])
    assert inspection["profile"]["embedded_font_count"] >= 4, path
    reader = PdfReader(path)
    page_texts = [compact(page.extract_text()) for page in reader.pages]
    joined = "".join(page_texts)
    for value in (data["number"], data["customer"], data["issued"], data["due"]):
        assert compact(value) in joined, (path, "missing invoice field", value)
    positions = []
    item_pages = set()
    for item in data["items"]:
        marker = compact(item["description"])
        assert joined.count(marker) == 1, (path, "missing or duplicate item", marker)
        positions.append(joined.index(marker))
        pages = [number for number, text in enumerate(page_texts) if marker in text]
        assert len(pages) == 1, (path, "item crosses pages", marker)
        item_pages.add(pages[0])
        if item.get("detail"):
            assert compact(item["detail"]) in page_texts[pages[0]], (path, "detached detail", marker)
    assert positions == sorted(positions), (path, "item order changed")
    total = sum((Decimal(item["unit_price"]) * item["quantity"] for item in data["items"]), Decimal("0.00"))
    assert f"USD{total:,.2f}" in page_texts[-1], (path, "total missing from last page")
    assert "Paymentinstructionsaresuppliedseparately." in page_texts[-1], (path, "payment note missing")
    for number, text in enumerate(page_texts, 1):
        assert text.count(f"Page{number}of{len(reader.pages)}") == 1, (path, "page counter", number)
        if number - 1 in item_pages:
            assert "DESCRIPTIONHOURSRATEAMOUNT" in text, (path, "table heading missing", number)

    page_bounds = []
    with pdfium.PdfDocument(path) as document:
        assert len(document) == len(reader.pages) == inspection["page_count"]
        for number in range(len(document)):
            page = document[number]
            width, height = page.get_size()
            assert abs(width - 595.276) < 0.1 and abs(height - 841.890) < 0.1, (path, "not A4")
            text = page.get_textpage()
            boxes = []
            try:
                for index in range(text.count_chars()):
                    if not text.get_text_range(index, 1).strip():
                        continue
                    left, bottom, right, top = text.get_charbox(index)
                    assert left >= -0.25 and bottom >= -0.25 and right <= width + 0.25 and top <= height + 0.25, (
                        path, "text outside the visible page", number + 1, index, (left, bottom, right, top))
                    boxes.append((left, bottom, right, top))
                assert boxes, (path, "empty page", number + 1)
                page_bounds.append({"page": number + 1, "characters": len(boxes),
                                    "text_box": [min(b[0] for b in boxes), min(b[1] for b in boxes),
                                                 max(b[2] for b in boxes), max(b[3] for b in boxes)]})
            finally:
                text.close()
                page.close()
    return {"page_count": len(reader.pages), "item_count": len(data["items"]),
            "total": f"{total:,.2f}", "all_text_within_page": True,
            "ordered_items_once": True, "repeated_table_headers": True,
            "page_counters": True, "page_bounds": page_bounds,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def check_layouts(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    short = load_invoice("INV-1042")
    long = load_invoice("INV-1043")
    many = load_invoice("INV-1043")
    many["number"] = "INV-MANY-80"
    many["items"] = [{**long["items"][0], "description": f"Service line {i:03d}"} for i in range(1, 81)]
    wrapped = load_invoice("INV-1043")
    wrapped["number"] = "INV-WRAPPED"
    wrapped["customer"] = "North <East> & ${rows} — Research and Development Studio"
    wrapped["items"] = [
        {**item, "description": item["description"] + " — planning, design, implementation and review",
         "detail": "Discuss requirements, prepare the design, implement the agreed changes, and document the handoff. " * 3}
        for item in wrapped["items"][:12]
    ]
    empty = load_invoice("INV-1042")
    empty["items"] = []
    results = []
    for name, data in [("short", short), ("long", long), ("many", many), ("wrapped", wrapped), ("empty", empty)]:
        pdf = render_invoice(data)
        path = out / f"{name}.pdf"
        path.write_bytes(pdf)
        result = inspect_invoice(path, data)
        assert result["page_count"] == 1 if name in {"short", "empty"} else result["page_count"] > 1, (name, result)
        assert pdf == render_invoice(data), (name, "repeat render differs")
        previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages_to_dir(
            str(path), str(out / name), 110, "invoice")
        result.update(case=name, repeat_render_identical=True, previews=previews)
        results.append(result)
    report = {"ok": True, "cases": results,
              "scope": "Synthetic invoice fixtures; independent text and glyph bounds, page numbers, row order, headings, totals and native previews."}
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    print(json.dumps(check_layouts(parser.parse_args().out), indent=2))
