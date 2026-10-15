#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise component documents through the installed wheel and an independent PDF reader."""
from __future__ import annotations

import argparse
from copy import deepcopy
from hashlib import sha256
from importlib import metadata, resources
from io import BytesIO
import json
from pathlib import Path
import runpy
import sys

import fullbleed
from fullbleed.ui import Document, Heading, LayoutGrid, Row, Stack, StyleSheet, Text
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "examples/components"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "target/component-authoring")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    # Only the example's local style module is added. fullbleed was already
    # imported from the installed distribution, never from python/.
    assert ROOT / "python/fullbleed" not in Path(fullbleed.__file__).parents
    sys.path.insert(0, str(EXAMPLE))
    try:
        invoice = runpy.run_path(str(EXAMPLE / "invoice.py"))["Invoice"]
    finally:
        sys.path.remove(str(EXAMPLE))
    original = json.loads((EXAMPLE / "data.json").read_text(encoding="utf-8"))
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    cases = []
    variants = [("invoice", original, ["Maple & Finch", "$8,100.00"], ["settled", "False"])]
    escaped = deepcopy(original)
    escaped["customer"] = '<strong>Literal & "quoted"</strong>'
    variants.append(("escaped-data", escaped, [escaped["customer"]], []))
    paid = deepcopy(original)
    paid["paid"] = True
    variants.append(("paid", paid, ["Paid in full", "This invoice has been settled."], ["Amount due"]))
    empty = deepcopy(original)
    empty["items"] = []
    variants.append(("empty", empty, ["$0.00"], ["False"]))
    credit = deepcopy(original)
    credit["items"] = [{"name": "Credit", "detail": "A prior payment applied to this invoice.",
                        "quantity": 1, "rate_cents": -150, "progress": 100}]
    variants.append(("credit", credit, ["-$1.50"], ["-$2.50"]))
    long = deepcopy(original)
    long["items"] = [{**original["items"][i % 3], "name": f"Deliverable {i:03d}",
                      "detail": "Carefully scoped work, reviewed together and delivered with supporting notes."} for i in range(1, 61)]
    variants.append(("long-table", long, [item["name"] for item in long["items"]], ["False"]))
    for name, data, present, absent in variants:
        artifact = invoice(data)
        html = artifact.to_html(a11y_mode="raise")
        css = artifact.to_css()
        pdf = artifact.render_pdf(engine, a11y_mode="raise")
        assert pdf == artifact.render_pdf(engine), (name, "repeat output")
        assert pdf == engine.render_pdf(html, css), (name, "explicit HTML/CSS path")
        reported_pdf, missing = engine.render_pdf_with_glyph_report(html, css)
        assert reported_pdf == pdf and not missing, (name, "missing glyphs", missing)
        reader = PdfReader(BytesIO(pdf))
        text = " ".join(" ".join(page.extract_text() for page in reader.pages).split())
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "text.txt").write_text(text, encoding="utf-8")
        for expected in present:
            if expected == "Paid in full":
                # pypdf infers extra spaces between the label's tracked glyphs.
                assert "paidinfull" in "".join(text.casefold().split()), (name, "missing paid label")
            else:
                assert expected.casefold() in text.casefold(), (name, "missing text", expected)
        for forbidden in absent:
            assert "".join(forbidden.casefold().split()) not in "".join(text.casefold().split()), (name, "unexpected text", forbidden)
        if name == "long-table":
            assert len(reader.pages) > 1
            for expected in present:
                assert text.count(expected) == 1, (name, "duplicated row", expected)
        else:
            assert len(reader.pages) == 1, (name, len(reader.pages))
        artifact.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css")
        previews = artifact.render_image_pages(engine, dpi=96)
        assert len(previews) == len(reader.pages)
        for index, preview in enumerate(previews, 1):
            assert bytes(preview).startswith(b"\x89PNG\r\n\x1a\n")
            (folder / f"page-{index}.png").write_bytes(bytes(preview))
        cases.append({"name": name, "pages": len(reader.pages), "pdf_sha256": sha256(pdf).hexdigest(),
                      "repeated_pdf_equal": True, "direct_render_equal": True, "missing_glyphs": 0,
                      "expected_text_checks": len(present), "native_previews": len(previews)})

    @Document(styles=StyleSheet({"@page": {"size": "A5", "margin": "15mm"},
                                "body": {"font_family": "Inter", "font_size": "10pt"}}))
    def Layouts():
        return Stack(Heading("Reusable layouts", level=1),
                     Row(Text("ROW_LEFT"), Text("ROW_RIGHT")),
                     Stack(Text("STACK_TOP"), Text("STACK_BOTTOM")),
                     LayoutGrid(Text("GRID_LEFT"), Text("GRID_RIGHT")), gap="16pt")
    layout = Layouts()
    pdf = layout.render_pdf(engine)
    positions = {}
    def record(text, cm, tm, font_dict, font_size):
        for token in ("ROW_LEFT", "ROW_RIGHT", "STACK_TOP", "STACK_BOTTOM", "GRID_LEFT", "GRID_RIGHT"):
            if token in text:
                # These fixtures have no text rotation or scale transforms.
                positions[token] = [tm[4] + cm[4], tm[5] + cm[5]]
    reader = PdfReader(BytesIO(pdf))
    assert len(reader.pages) == 1
    reader.pages[0].extract_text(visitor_text=record)
    assert len(positions) == 6, positions
    for left, right in [("ROW_LEFT", "ROW_RIGHT"), ("GRID_LEFT", "GRID_RIGHT")]:
        assert positions[left][0] + 30 < positions[right][0], positions
        assert abs(positions[left][1] - positions[right][1]) < 1, positions
    assert positions["STACK_TOP"][1] > positions["STACK_BOTTOM"][1] + 5, positions
    (args.out / "layouts.pdf").write_bytes(pdf)
    (args.out / "layouts.png").write_bytes(bytes(layout.render_image_pages(engine, dpi=96)[0]))
    result = {"ok": True, "version": metadata.version("fullbleed"), "runtime": str(fullbleed.__file__),
              "cases": cases, "layout_positions": positions, "layout_pdf_sha256": sha256(pdf).hexdigest(),
              "scope": "Component serialization, data variants, deterministic PDF bytes, text, glyphs, preview generation and basic layout geometry. Not a PDF accessibility conformance assessment."}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "cases": len(cases) + 1, "pages": sum(case["pages"] for case in cases) + 1}))


if __name__ == "__main__":
    main()
