# SPDX-License-Identifier: MIT
"""Shared document code; no web-framework dependency."""

from decimal import Decimal
from html import escape
from importlib import resources
from pathlib import Path
from string import Template
from typing import Any

import fullbleed

ROOT = Path(__file__).resolve().parent

PDF_HEADERS = {
    "Content-Disposition": 'attachment; filename="invoice.pdf"',
    "Cache-Control": "private, no-store",
}


def load_invoice(invoice_id: str) -> dict[str, Any] | None:
    """Replace this fictional record with your application's authorized lookup."""
    if invoice_id != "INV-1042":
        return None
    return {
        "number": "INV-1042",
        "customer": "Maple & Finch",
        "issued": "2026-10-01",
        "due": "2026-10-31",
        "items": [
            {"description": "Design workshop", "detail": "Positioning, priorities, and a clear creative direction", "quantity": 8, "unit_price": "125.00"},
            {"description": "Implementation", "detail": "A cohesive visual system, from layout to launch", "quantity": 6, "unit_price": "95.00"},
            {"description": "Review and handoff", "detail": "Final refinements, documentation, and delivery", "quantity": 2, "unit_price": "150.00"},
        ],
    }


def render_invoice(data: dict[str, Any]) -> bytes:
    """Render application-owned invoice data into PDF bytes in memory."""
    amounts = [Decimal(item["unit_price"]) * item["quantity"] for item in data["items"]]
    total = sum(amounts, Decimal("0.00"))
    rows = "".join(
        f"<tr><td><strong>{escape(item['description'])}</strong>"
        f"<p class='item-note'>{escape(item.get('detail', ''))}</p></td>"
        f"<td class='number'>{escape(str(item['quantity']))}</td>"
        f"<td class='number'>{Decimal(item['unit_price']):,.2f}</td>"
        f"<td class='number'>{amount:,.2f}</td></tr>"
        for item, amount in zip(data["items"], amounts)
    )
    # Template substitution runs once: placeholder-like text inside a customer
    # value stays literal. Only application-owned row markup enters unescaped.
    html = Template((ROOT / "templates/invoice.html").read_text(encoding="utf-8")).substitute(
        number=escape(data["number"]), customer=escape(data["customer"]),
        issued=escape(data["issued"]), due=escape(data["due"]), rows=rows,
        total=f"{total:,.2f}",
    )
    css = (ROOT / "templates/invoice.css").read_text(encoding="utf-8")
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(
        font_files=[str(font), *(str(ROOT / "fonts" / name) for name in [
            "DMSerifDisplay-Regular.ttf", "DMSerifDisplay-Italic.ttf", "BebasNeue-Regular.ttf",
        ])],
        document_title=f"Invoice {data['number']}",
        document_lang="en",
    )
    return engine.render_pdf(html, css)
