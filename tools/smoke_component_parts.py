# SPDX-License-Identifier: MIT
"""Check explicit part forwarding against rendered controls from an installed wheel."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from html.parser import HTMLParser
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from fullbleed.ui.experimental import (
    AuthoringError, Box, Column, DataTable, Document, Flow, Heading, Page, Paragraph,
    Renderer, Theme, component, mm, pt, style, token,
)
from fullbleed.ui.experimental import bootstrap as bs
from PIL import ImageChops
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
BLUE, GREEN, RUST = "#315d9b", "#245541", "#a0442e"


class Markup(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.elements, self.inline_styles = [], []
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        self.elements.append(tag)
        if "style" in dict(attrs):
            self.inline_styles.append(dict(attrs)["style"])


@component(parts=("label",))
def PrivateLabel():
    return Paragraph("PRIVATE DETAIL", part="label", style=style(color=GREEN, font_size=pt(9)))


@component(parts=("label",), optional_parts=("note",))
def Field(*, color=BLUE, note=None, note_color=BLUE):
    return Flow(
        Paragraph("PAID IN FULL", part="label", style=style(color=color, font_size=pt(18), font_weight=700)),
        Paragraph(note, part="note", style=style(color=note_color)) if note else None,
        PrivateLabel(), gap=pt(6),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Install the candidate wheel first"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-16T00:00:00Z")
    theme = Theme({"accent": RUST}, text=style(font_family="Inter", font_size=pt(11), line_height=1.25))
    records = []

    def document(node):
        return Document(Flow(Heading("A considered record", level=1), node, gap=pt(18)),
                        title="Component part contract", lang="en-US", page=Page("A5", margin=mm(15)), theme=theme)

    def check(name, actual, control, *, min_pages=1, expected_text=("PAIDINFULL",)):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        authored, expected = document(actual), document(control)
        plan = authored.compile(target="pdf", a11y_mode="raise")
        prepared = authored.prepare(renderer, a11y_mode="raise")
        pdf, reference = prepared.render_pdf(), expected.render_pdf(renderer, a11y_mode="raise")
        assert pdf == authored.render_pdf(renderer), (name, "repeat output")
        _, missing = renderer.engine_for(authored).render_pdf_with_glyph_report(plan.html, plan.css)
        assert not missing, (name, missing)
        reader = PdfReader(BytesIO(pdf))
        assert len(reader.pages) >= min_pages
        box = reader.pages[0].mediabox
        assert abs(float(box.width) - 419.528) < .01 and abs(float(box.height) - 595.276) < .01
        text = "\n".join("".join(page.extract_text().split()) for page in reader.pages)
        assert all(value in text for value in expected_text), (name, text)
        assert reader.metadata.title == authored.title and reader.trailer["/Root"]["/Lang"] == authored.lang
        with closing(pdfium.PdfDocument(pdf)) as actual_pdf, closing(pdfium.PdfDocument(reference)) as expected_pdf:
            assert len(actual_pdf) == len(expected_pdf) == len(reader.pages)
            for index in range(len(actual_pdf)):
                with closing(actual_pdf[index]) as actual_page, closing(expected_pdf[index]) as expected_page:
                    with closing(actual_page.render(scale=1.5)) as actual_bitmap, closing(expected_page.render(scale=1.5)) as expected_bitmap:
                        image = actual_bitmap.to_pil().convert("RGB")
                        expected_image = expected_bitmap.to_pil().convert("RGB")
                        assert image.size == expected_image.size
                        assert ImageChops.difference(image, expected_image).getbbox() is None, (name, index, "control pixels")
                        image.save(folder / f"independent-{index + 1}.png")
        styled, plain = actual.to_html(include_styles=True, theme=theme), actual.to_html(include_styles=False)
        assert not Markup(plain).inline_styles and "<style" not in plain
        # Virtual component boundaries and forwarding must add no layout boxes.
        assert Markup(actual.to_html(theme=theme)).elements == Markup(control.to_html(theme=theme)).elements
        for filename, value in (("document.html", plan.html), ("document.css", plan.css),
                                ("fragment-styled.html", styled), ("fragment-plain.html", plain)):
            (folder / filename).write_text(value, encoding="utf-8", newline="\n")
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "control.pdf").write_bytes(reference)
        (folder / "native.png").write_bytes(prepared.render_image_pages(dpi=96)[0])
        records.append({"name": name, "pdf_sha256": sha256(pdf).hexdigest(),
                        "control_pdf_sha256": sha256(reference).hexdigest(), "control_pixels_equal": True,
                        "repeat_bytes_equal": True, "missing_glyphs": 0, "pages": len(reader.pages),
                        "page_size_pt": [float(box.width), float(box.height)], "no_extra_elements": True})
        return reader

    calls = []

    @component(parts=("heading",), optional_parts=("details",))
    def Summary(*, wrap, note=None):
        calls.append((wrap, note))
        field = Field(note=note).forward_parts(heading="label", details="note")
        return Box(field) if wrap else field

    for wrap in (False, True):
        base = Summary(wrap=wrap)
        actual = base.with_parts(heading=style(color=token("accent")), details=style(color=token("unused")))
        control = Field(color=RUST)
        check("wrapped" if wrap else "direct", actual, Box(control) if wrap else control)
        assert base.required_tokens == () and actual.required_tokens == ("accent",)
    check("optional-present", Summary(wrap=True, note="RECEIPT ENCLOSED").with_parts(details=style(color=GREEN)),
          Box(Field(note="RECEIPT ENCLOSED", note_color=GREEN)), expected_text=("PAIDINFULL", "RECEIPTENCLOSED"))
    assert calls == [(False, None), (True, None), (True, "RECEIPT ENCLOSED")], calls

    shared = Field()
    check("shared-private", Flow(shared.forward_parts("label"), shared, gap=pt(18)).with_parts(label=style(color=RUST)),
          Flow(Field(color=RUST), Field(), gap=pt(18)))
    assert shared.required_tokens == ()

    @component(parts=("headings",), optional_parts=("values", "empty"))
    def Ledger(rows):
        return Box(DataTable(rows, columns=[Column("RECORD", str)], empty="No billable items.").forward_parts(
            headings="header", values="cell", empty="empty"))

    rows = tuple(f"RECORD_{index:03d}" for index in range(80))
    headings, values = style(background="#e7edf4", color=BLUE), style(color=RUST, padding=pt(6))
    table = Ledger(rows).with_parts(headings=headings, values=values)
    control = Box(DataTable(rows, columns=[Column("RECORD", str)]).with_parts(header=headings, cell=values))
    reader = check("paginated-table", table, control, min_pages=2, expected_text=rows)
    text = "\n".join(page.extract_text() for page in reader.pages)
    assert all(text.count(row) == 1 for row in rows)
    assert all(page.extract_text().lstrip().startswith("RECORD") for page in reader.pages[1:])
    empty = Ledger(()).with_parts(values=style(color=token("unused")), empty=style(color=GREEN))
    check("empty-table", empty, Box(DataTable((), columns=[Column("RECORD", str)], empty="No billable items.")
          .with_parts(empty=style(color=GREEN))), expected_text=("Nobillableitems.",))
    assert empty.required_tokens == ()

    @component(parts=("heading",), optional_parts=("footer",))
    def KitCard():
        return Box(bs.Card("PAID IN FULL", title="Receipt").forward_parts(heading="title", footer="footer"))

    check("bootstrap-card", KitCard().with_parts(heading=style(color=RUST)),
          Box(bs.Card(bs.CardBody(bs.CardTitle("Receipt", style=style(color=RUST)), "PAID IN FULL"), body=False)))

    @component(parts=("label",))
    def Broken():
        return Box(Field())

    try:
        Broken()
    except AuthoringError as error:
        assert error.code == "component.part_missing" and error.path == ("Broken", "label")
        diagnostic = {"code": error.code, "path": list(error.path), "message": error.message}
    else:
        raise AssertionError("Missing required binding silently accepted")
    report = {"ok": True, "runtime": str(fullbleed.__file__), "checks": records, "missing_binding": diagnostic,
              "callbacks_captured_once": True,
              "scope": "Seven part-forwarding fixtures checked against rendered controls with an independent PDF reader. Includes a multi-page table, private and shared children, Bootstrap and styled/unstyled HTML; not arbitrary component or CSS parity."}
    (args.out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rendered_controls": len(records), "pages": sum(item["pages"] for item in records)}))


if __name__ == "__main__":
    main()
