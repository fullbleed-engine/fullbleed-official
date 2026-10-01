# SPDX-License-Identifier: MIT
"""Run: python -m flask --app flask_app run --host 127.0.0.1 --port 8000"""

from flask import Flask, Response, abort

from invoice import PDF_HEADERS, load_invoice, render_invoice


app = Flask(__name__)


@app.get("/invoices/<invoice_id>.pdf")
def invoice_pdf(invoice_id: str) -> Response:
    invoice = load_invoice(invoice_id)
    if invoice is None:
        abort(404)
    return Response(render_invoice(invoice), mimetype="application/pdf", headers=PDF_HEADERS)
