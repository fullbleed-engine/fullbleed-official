# SPDX-License-Identifier: MIT
"""Northstar Studio: reusable Python components, no authored HTML strings."""
from __future__ import annotations

from decimal import Decimal

from fullbleed.ui.experimental import (
    Box, Column, DataTable, Document, Fragment, Header, Heading, Node,
    Paragraph, Section, Strong, Text, component, pct, pt, style,
)
from component_primitives import Folio, Lines, design_document, money


@component
def StudioMasthead() -> Node:
    return Header(
        Box(Text("Northstar"), Text("Studio"),
            Node("small", ("DESIGN WITH DIRECTION",)), class_name="brand"),
        Box(Lines("Brand", "Digital", "Experiences", "That matter."), class_name="brand-note kicker"),
        Box(Lines("42 Cedar Avenue", "Portland, OR 97205", "hello@example.invalid",
                  "northstar.example.invalid"), class_name="address"),
        class_name="masthead",
    )


@component(parts=("label", "value"))
def InvoiceField(label: str, *lines) -> Node:
    return Box(Heading(label, class_name="kicker", part="label"),
               Paragraph(Lines(*lines), part="value"))


@component
def InvoiceItems(items) -> Node:
    return DataTable(
        items,
        columns=(
            Column("DESCRIPTION", lambda item: Fragment(
                Strong(item["description"]), Paragraph(item["detail"], class_name="item-note")), width=pct(56)),
            Column("HOURS", lambda item: item["hours"], align="right"),
            Column("RATE", lambda item: money(item["rate"]), align="right"),
            Column("AMOUNT", lambda item: money(Decimal(item["rate"]) * item["hours"]), align="right"),
        ),
        attrs={"aria-label": "Invoice line items"},
        style=style(table_layout="auto"),
    ).with_parts(
        header=style(padding=(pt(10), 0), border_bottom=".55pt solid #bdc8bf", font_weight=600),
        cell=style(padding=(pt(12), 0), border_bottom=".55pt solid #bdc8bf"),
    )


@component
def SummaryLine(label: str, amount: str) -> Node:
    return Box(Text(label), Text(amount), class_name="summary-line")


@component(parts=("label", "amount"))
def AmountDue(amount: Decimal) -> Node:
    return Box(Box("TOTAL DUE · USD", class_name="kicker", part="label"),
               Box(money(amount), class_name="value", part="amount"), class_name="total")


@component
def PaymentNote(title: str, *lines) -> Node:
    return Box(Heading(title, class_name="kicker", style=style(margin_bottom=pt(6))),
               Paragraph(Lines(*lines)))


def northstar_invoice(items) -> Document:
    """Capture line items once; calculate all amounts with Decimal."""
    items = tuple(items)
    subtotal = sum((Decimal(item["rate"]) * item["hours"] for item in items), Decimal("0"))
    return design_document(
        Section(
            StudioMasthead(),
            Box(Heading("INVOICE", level=1),
                Paragraph(Lines("GOOD WORK.", "CLEAR DETAILS.", "LET'S MAKE", "WHAT'S NEXT."),
                          class_name="title-note"), class_name="title-row"),
            Section(
                InvoiceField("Prepared for", Strong("Maple & Finch"), "Attn. Avery Morgan", "Portland, Oregon"),
                InvoiceField("The project", "Autumn launch", "Brand workshop, digital", "design & handoff"),
                InvoiceField("Invoice NS-1042", "Issued · 01 October 2026", "Due · 31 October 2026", "Currency · USD"),
                class_name="details grid-3",
            ),
            InvoiceItems(items),
            Section(
                Box(Paragraph(Lines("A little clarity.", "A lot of possibility."), class_name="thanks"),
                    Paragraph(Lines("Thank you for inviting us", "to help shape your next chapter."),
                              class_name="small muted", style=style(margin_top=pt(14)))),
                Box(SummaryLine("Subtotal", money(subtotal)), SummaryLine("Tax · 0% in this sample", "$0.00"),
                    AmountDue(subtotal)),
                class_name="amounts",
            ),
            Section(
                PaymentNote("Payment details", "Reference NS-1042 with your payment.",
                            "Payment instructions are supplied separately."),
                PaymentNote("A note from the studio", "Questions about a line item?", "Write to hello@example.invalid."),
                class_name="payment",
            ),
            Folio("NORTHSTAR STUDIO / INVOICE NS-1042", "FICTIONAL SAMPLE · 01 / 01"),
            class_name="page invoice",
        ),
        kind="invoice", title="Northstar Studio — Invoice NS-1042",
    )
