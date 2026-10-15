# SPDX-License-Identifier: MIT
"""A data-driven invoice composed from reusable Python functions."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata, resources
import json
from pathlib import Path

import fullbleed
from fullbleed.ui import (
    Document, component, Box, Fragment, Header, Footer, Heading, LayoutGrid,
    LineBreak, Paragraph, Section, Strong, Table, TableCaption, THead, TBody,
    Tr, Th, Td, Text, pct, style,
)
from styles import INVOICE_STYLES

ROOT = Path(__file__).resolve().parent


def money(cents: int) -> str:
    sign = "-" if cents < 0 else ""
    value = abs(cents)
    return f"{sign}${value // 100:,}.{value % 100:02d}"


@component
def Field(label: str, value: str):
    return Box(Text(label, class_name="label"), Paragraph(value, class_name="value"))


@component
def WorkRow(item: dict):
    return Tr(
        Td(Paragraph(item["name"], class_name="item-name"),
           Paragraph(item["detail"], class_name="detail"),
           Box(Box(class_name="progress-fill", style=style(width=pct(item["progress"]))),
               class_name="progress-track", aria_hidden=True), class_name="work"),
        Td(item["quantity"], class_name="number hours"),
        Td(money(item["rate_cents"]), class_name="number rate"),
        Td(money(item["quantity"] * item["rate_cents"]), class_name="number amount"))


@component
def WorkTable(items: list[dict]):
    return Table(
        TableCaption("The work, in detail"),
        THead(Tr(
            Th("Project", class_name="work", scope="col"),
            Th("Hours", class_name="number hours", scope="col"),
            Th("Rate", class_name="number rate", scope="col"),
            Th("Amount", class_name="number amount", scope="col"))),
        TBody(WorkRow(item) for item in items))


@component
def Total(items: list[dict], due: str, paid: bool):
    cents = sum(item["quantity"] * item["rate_cents"] for item in items)
    return Section(
        Text("Paid in full" if paid else "Amount due · USD", class_name="label"),
        Paragraph(money(cents), class_name="total-amount"),
        Paragraph("Thank you." if paid else f"Due {due}", class_name="total-note"),
        class_name="total", aria_label="Invoice total")


@component
def PaymentNote(paid: bool):
    return Box(
        Paragraph(Strong("Onward, together."),
                  "Please include the invoice number with your payment. We’re grateful to be part of what comes next.", class_name="note"),
        paid and Paragraph("This invoice has been settled.", style=style(font_size="8pt", margin_top="6pt")),
    )


@Document(title="Northstar Studio / Invoice", bootstrap=False, lang="en", styles=INVOICE_STYLES)
def Invoice(data: dict):
    return Fragment(
        Header(Paragraph("NORTHSTAR / STUDIO", class_name="wordmark"),
           Paragraph("Independent design.\nThoughtful by nature.", class_name="brand-note"), class_name="brand"),
        Section(Paragraph(f'Invoice {data["number"]} · {data["period"]}', class_name="eyebrow"),
           Heading("Good work.", LineBreak(), "Clear numbers.", level=1),
           Paragraph("A shared vision, made tangible. Here’s the work we brought to life together.", class_name="lede"), class_name="intro"),
        LayoutGrid(Field("Prepared for", data["customer"]), Field("Issued", data["issued"]),
           Field("Payment due", data["due"]), class_name="meta", aria_label="Invoice details"),
        WorkTable(data["items"]),
        LayoutGrid(PaymentNote(data["paid"]),
           Total(data["items"], data["due"], data["paid"]), class_name="summary", aria_label="Payment summary"),
        Footer("NORTHSTAR STUDIO  /  FICTIONAL DEMONSTRATION  /  GENERATED WITH FULLBLEED", class_name="footer"),
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "output")
    args = parser.parse_args()
    data = json.loads((ROOT / "data.json").read_text(encoding="utf-8"))
    invoice = Invoice(data)
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)])
    pdf = invoice.render_pdf(engine, a11y_mode="raise")
    assert pdf == invoice.render_pdf(engine), "Repeated render changed"
    args.output.mkdir(parents=True, exist_ok=True)
    invoice.emit_artifacts(html_path=args.output / "invoice.html", css_path=args.output / "invoice.css")
    (args.output / "invoice.pdf").write_bytes(pdf)
    pages = invoice.render_image_pages(engine, dpi=96)
    for index, page in enumerate(pages, 1):
        (args.output / f"page-{index}.png").write_bytes(bytes(page))
    report = {"ok": True, "version": metadata.version("fullbleed"), "pages": len(pages),
              "pdf_sha256": sha256(pdf).hexdigest(), "repeated_pdf_equal": True}
    (args.output / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
