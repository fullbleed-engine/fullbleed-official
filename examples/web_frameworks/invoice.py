# SPDX-License-Identifier: MIT
"""Shared document code; no web-framework dependency."""

from decimal import Decimal
from html import escape
from importlib import resources
from typing import Any

import fullbleed


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
            {"description": "Design workshop", "quantity": 8, "unit_price": "125.00"},
            {"description": "Implementation", "quantity": 6, "unit_price": "95.00"},
            {"description": "Review and handoff", "quantity": 2, "unit_price": "150.00"},
        ],
    }


def render_invoice(data: dict[str, Any]) -> bytes:
    """Render application-owned invoice data into PDF bytes in memory."""
    amounts = [Decimal(item["unit_price"]) * item["quantity"] for item in data["items"]]
    total = sum(amounts, Decimal("0.00"))
    rows = "".join(
        f"<tr><td>{escape(item['description'])}</td>"
        f"<td class='number'>{item['quantity']}</td>"
        f"<td class='number'>USD {Decimal(item['unit_price']):,.2f}</td>"
        f"<td class='number'>USD {amount:,.2f}</td></tr>"
        for item, amount in zip(data["items"], amounts)
    )
    html = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Invoice</title></head>
<body><main>
  <p class="brand">Northstar Studio</p>
  <h1>Invoice {escape(data['number'])}</h1>
  <p class="dates">Issued {escape(data['issued'])} · Due {escape(data['due'])}</p>
  <div class="recipient"><p class="label">BILL TO</p>
    <p>{escape(data['customer'])}</p></div>
  <table><thead><tr><th>Description</th><th class="number">Qty</th>
    <th class="number">Unit price</th><th class="number">Amount</th></tr></thead>
    <tbody>{rows}</tbody></table>
  <p class="total">Total due: USD {total:,.2f}</p>
  <p class="note">Thank you for your business.</p>
</main></body></html>"""
    css = """
@page { size: A4; margin: 20mm; }
body { font-family: Inter, sans-serif; color: #18312e; font-size: 10pt;
       line-height: 1.45; margin: 0; }
.brand { color: #175c52; font-size: 13pt; font-weight: 700; margin: 0 0 24pt; }
h1 { font-size: 27pt; margin: 0 0 6pt; }
.dates, .label, .note { color: #526762; }
.dates { margin: 0 0 25pt; }
.recipient { margin-bottom: 24pt; }
.recipient p { margin: 0 0 4pt; }
.label { font-size: 8pt; font-weight: 700; }
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: 10pt 8pt; border-bottom: 0.6pt solid #d6e2df; }
th { background: #edf4f1; color: #175c52; font-size: 9pt; }
.number { text-align: right; white-space: nowrap; }
.total { margin: 24pt 0 0; text-align: right; font-size: 14pt; }
.note { margin-top: 42pt; font-size: 9pt; }
"""
    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(
        font_files=[str(font)],
        document_title=f"Invoice {data['number']}",
        document_lang="en",
    )
    return engine.render_pdf(html, css)
