# SPDX-License-Identifier: MIT
"""Run: python -m uvicorn fastapi_app:app --host 127.0.0.1 --port 8000"""

from fastapi import FastAPI, HTTPException, Response

from invoice import PDF_HEADERS, load_invoice, render_invoice


class PDFResponse(Response):
    media_type = "application/pdf"


app = FastAPI(title="Fullbleed invoice example")


@app.get("/invoices/{invoice_id}.pdf", response_class=PDFResponse)
def invoice_pdf(invoice_id: str) -> PDFResponse:
    invoice = load_invoice(invoice_id)
    if invoice is None:
        raise HTTPException(status_code=404, detail="Invoice not found")
    return PDFResponse(render_invoice(invoice), headers=PDF_HEADERS)
