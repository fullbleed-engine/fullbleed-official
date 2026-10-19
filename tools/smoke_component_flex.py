# SPDX-License-Identifier: MIT
"""Measure static component and semantic flex layouts with an independent PDF reader."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

from fullbleed.ui.experimental import Document, Heading, Page, Paragraph, Renderer, Text, Theme, pt, style
from fullbleed.ui.experimental import bootstrap as bs
from pypdf import PdfReader
import pypdfium2 as pdfium


def authored():
    shared = dict(page=Page((pt(480), pt(160)), margin=pt(20)),
                  theme=Theme(text=style(font_family="Inter", font_size=pt(10))))
    for linked in (False, True):
        for large in (False, True):
            brand = bs.NavbarBrand("Northstar Studio", href="https://example.invalid/orders" if linked else None)
            if large:
                brand = brand.with_style(font_size=pt(22))
            node = bs.Navbar(brand, bs.NavbarNav(bs.NavItem(bs.NavLink("Orders", active=True))), expand=True)
            name = f"navbar-{'link' if linked else 'text'}-{'large' if large else 'default'}"
            yield name, Document(node, **shared), "Northstar Studio", "Orders", 8.0
    for large in (False, True):
        node = bs.Toast("The document is ready.", title="Exported", subtitle="Today")
        if large:
            node = node.with_style(font_size=pt(16))
        yield f"toast-{'large' if large else 'default'}", Document(node, **shared), "Exported", "Today", 3.0
    for component in (Paragraph, Heading):
        for display in ("flex", "inline-flex"):
            node = component(Text("I", class_name="flex-marker", style=style(width=pt(12), margin_right=pt(6))),
                Text("Value", class_name="flex-value"), class_name="semantic-row",
                style=style(display=display, margin=0, font_size=pt(10), font_weight=400))
            yield f"{component.__name__.lower()}-{display}", Document(node, **shared), "I", "Value", 8.0


def label_bounds(text, label):
    with closing(text.search(label)) as search:
        match = search.get_next()
        assert match is not None, label
        assert search.get_next() is None, f"Repeated label: {label}"
    start, count = match
    assert text.get_text_range(start, count) == label
    boxes = [text.get_charbox(i) for i in range(start, start + count)]
    return [min(b[0] for b in boxes), min(b[1] for b in boxes),
            max(b[2] for b in boxes), max(b[3] for b in boxes)]


def check(out):
    out.mkdir(parents=True, exist_ok=True)
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-17T00:00:00Z")
    records = []
    for name, document, first, second, minimum_gap in authored():
        folder = out / name
        folder.mkdir(exist_ok=True)
        source = document.compile(target="pdf")
        engine = renderer.engine_for(document)
        prepared = document.prepare(renderer)
        pdf = prepared.render_pdf()
        assert pdf == document.render_pdf(renderer) == prepared.render_pdf()
        styled_html = document.to_html(include_styles=True)
        plain_html = document.to_html(include_styles=False)
        assert "<script" not in styled_html.lower() and "<script" not in plain_html.lower()
        assert "<style" not in plain_html and " style=" not in plain_html
        exported_pdf = bytes(engine.render_pdf(styled_html, ""))
        # NavLink has a print representation. Compare browser HTML with its own
        # compiled source, then measure spacing in both representations.
        html_source = document.compile(target="html")
        assert exported_pdf == bytes(engine.render_pdf(html_source.html, html_source.css))
        reader = PdfReader(BytesIO(pdf))
        assert len(reader.pages) == 1
        assert first in reader.pages[0].extract_text() and second in reader.pages[0].extract_text(), (name, reader.pages[0].extract_text())
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "styled-export.pdf").write_bytes(exported_pdf)
        (folder / "styled.html").write_text(styled_html, encoding="utf-8", newline="\n")
        (folder / "unstyled.html").write_text(plain_html, encoding="utf-8", newline="\n")
        layouts = {}
        for mode, data in (("document", pdf), ("styled-export", exported_pdf)):
            with closing(pdfium.PdfDocument(data)) as parsed, closing(parsed[0]) as page:
                assert len(parsed) == 1
                with closing(page.get_textpage()) as text:
                    left, right = label_bounds(text, first), label_bounds(text, second)
                with closing(page.render(scale=96 / 72)) as bitmap:
                    bitmap.to_pil().save(folder / (mode + "-pdfium.png"))
            gap = right[0] - left[2]
            same_row = min(left[3], right[3]) > max(left[1], right[1])
            layouts[mode] = dict(first_bounds_pt=left, second_bounds_pt=right, gap_pt=gap,
                minimum_gap_pt=minimum_gap, same_row=same_row, ok=same_row and gap >= minimum_gap)
        for mode, previews in (("native", prepared.render_image_pages(dpi=96)),
                ("finalized", engine.render_finalized_pdf_image_pages(str(folder / "document.pdf"), 96))):
            assert len(previews) == 1
            (folder / (mode + ".png")).write_bytes(bytes(previews[0]))
        records.append(dict(name=name, layouts=layouts, ok=all(r["ok"] for r in layouts.values()),
            pdf_sha256=sha256(pdf).hexdigest(), deterministic=True, styled_export_matches_html_plan=True,
            pdf_and_html_bytes_equal=pdf == exported_pdf))
        assert document.compile(target="pdf") == source
    result = dict(ok=all(row["ok"] for row in records), version=version("fullbleed"), cases=records,
        scope="Ten static Navbar, Toast, Paragraph and Heading layouts, with independent PDF text bounds; not complete flexbox or Bootstrap conformance.")
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps(dict(ok=result["ok"], version=result["version"], cases=len(result["cases"]),
        failed=[r["name"] for r in result["cases"] if not r["ok"]])))
    raise SystemExit(0 if result["ok"] else 1)


if __name__ == "__main__":
    main()
