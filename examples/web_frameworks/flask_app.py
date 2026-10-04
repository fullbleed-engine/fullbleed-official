# SPDX-License-Identifier: MIT
"""Run: python -m flask --app flask_app run --host 127.0.0.1 --port 8000"""

from flask import Flask, Response, abort

from demo import DEMO_HEADERS, page_html, preview_bytes
from invoice import PDF_HEADERS, load_invoice, render_invoice


app = Flask(__name__)


@app.get("/")
def index() -> Response:
    return Response(page_html(), mimetype="text/html", headers=DEMO_HEADERS)


@app.get("/preview.png")
def preview() -> Response:
    return Response(preview_bytes(), mimetype="image/png", headers=DEMO_HEADERS)


@app.get("/invoices/<invoice_id>.pdf")
def invoice_pdf(invoice_id: str) -> Response:
    invoice = load_invoice(invoice_id)
    if invoice is None:
        abort(404)
    return Response(render_invoice(invoice), mimetype="application/pdf", headers=PDF_HEADERS)
