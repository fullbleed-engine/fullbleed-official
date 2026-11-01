from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from html import escape
from importlib.resources import files
from pathlib import Path
from string import Template

import fullbleed


ROOT = Path(__file__).resolve().parent
DATA_PATH = ROOT / "data.json"
TEMPLATE_HTML_PATH = ROOT / "templates" / "invoice.html"
TEMPLATE_CSS_PATH = ROOT / "templates" / "invoice.css"
ITEM_HTML_PATH = ROOT / "templates" / "item.html"
OUTPUT_DIR = ROOT / "output"
HTML_PATH = OUTPUT_DIR / "invoice.html"
CSS_PATH = OUTPUT_DIR / "invoice.css"
PDF_PATH = OUTPUT_DIR / "invoice.pdf"
RUN_REPORT_PATH = OUTPUT_DIR / "invoice_run_report.json"
CENT = Decimal("0.01")


def number(value: object, label: str) -> Decimal:
    try:
        result = Decimal(str(value))
    except (InvalidOperation, ValueError) as exc:
        raise ValueError(f"{label} must be a finite decimal number.") from exc
    if not result.is_finite():
        raise ValueError(f"{label} must be a finite decimal number.")
    return result


def invoice_context(data: dict) -> tuple[dict[str, str], dict]:
    items = data.get("items")
    if not isinstance(items, list) or not items:
        raise ValueError("data.json must contain a non-empty items array.")
    rate = number(data.get("tax_rate", "0"), "tax_rate")
    if not Decimal(0) <= rate <= Decimal(1):
        raise ValueError("tax_rate must be a decimal fraction between 0 and 1.")
    row_template = Template(ITEM_HTML_PATH.read_text(encoding="utf-8"))
    rows = []
    subtotal = Decimal(0)
    for index, item in enumerate(items, 1):
        quantity = number(item["quantity"], f"items[{index}].quantity")
        price = number(item["unit_price"], f"items[{index}].unit_price")
        if quantity <= 0:
            raise ValueError(f"items[{index}].quantity must be greater than zero.")
        amount = (quantity * price).quantize(CENT, rounding=ROUND_HALF_UP)
        subtotal += amount
        rows.append(row_template.substitute(
            description=escape(str(item["description"]), quote=True),
            quantity=format(quantity.normalize(), "f"),
            unit_price=f"{price:,.{max(2, -price.as_tuple().exponent)}f}", amount=f"{amount:,.2f}",
        ))
    tax = (subtotal * rate).quantize(CENT, rounding=ROUND_HALF_UP)
    total = subtotal + tax
    context = {key: escape(str(data[key]), quote=True) for key in (
        "company_name", "company_address", "invoice_number", "invoice_date", "due_date",
        "customer_name", "customer_address", "currency", "payment_note",
    )}
    context.update(rows="\n".join(rows), subtotal=f"{subtotal:,.2f}",
                   tax_percent=format((rate * 100).normalize(), "f"),
                   tax=f"{tax:,.2f}", total=f"{total:,.2f}")
    return context, {"item_count": len(items), "currency": str(data["currency"]),
                     "subtotal": f"{subtotal:.2f}", "tax": f"{tax:.2f}", "total": f"{total:.2f}"}


def create_engine() -> fullbleed.PdfEngine:
    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0.5in",
        document_lang="en",
        document_title="Invoice",
        font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
    )
    if hasattr(engine, "document_css_href"):
        engine.document_css_href = CSS_PATH.name
    if hasattr(engine, "document_css_source_path"):
        engine.document_css_source_path = str(TEMPLATE_CSS_PATH)
    if hasattr(engine, "document_css_media"):
        engine.document_css_media = "all"
    if hasattr(engine, "document_css_required"):
        engine.document_css_required = True
    return engine


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    data = json.loads(DATA_PATH.read_text(encoding="utf-8"), parse_float=Decimal)
    context, calculated = invoice_context(data)
    html = Template(TEMPLATE_HTML_PATH.read_text(encoding="utf-8")).substitute(context)
    css = TEMPLATE_CSS_PATH.read_text(encoding="utf-8")

    engine = create_engine()
    html_css_emit_status = "ok"
    emitted: dict[str, object] = {}
    if hasattr(engine, "emit_artifacts"):
        emitted = dict(
            engine.emit_artifacts(
                html,
                css,
                str(HTML_PATH),
                str(CSS_PATH),
                False,
            )
        )
    else:
        html_css_emit_status = "fallback (native emitter unavailable)"
        HTML_PATH.write_text(html, encoding="utf-8")
        CSS_PATH.write_text(css, encoding="utf-8")

    pdf_bytes = int(engine.render_pdf_to_file(html, css, str(PDF_PATH)))
    previews = engine.render_finalized_pdf_image_pages_to_dir(
        str(PDF_PATH), str(OUTPUT_DIR / "preview"), 120, "invoice"
    )
    metadata = {}
    if hasattr(engine, "document_metadata"):
        metadata = dict(engine.document_metadata())

    run_report = {
        "schema": "fullbleed.new_template.invoice.run.v1",
        "ok": True,
        "html_path": str(HTML_PATH),
        "css_path": str(CSS_PATH),
        "css_source_path": str(TEMPLATE_CSS_PATH),
        "pdf_path": str(PDF_PATH),
        "pdf_bytes": pdf_bytes,
        "data_path": str(DATA_PATH),
        "calculated": calculated,
        "preview_paths": list(previews),
        "html_css_emit_status": html_css_emit_status,
        "document_css_href": metadata.get("document_css_href"),
        "document_css_source_path": metadata.get("document_css_source_path"),
        "document_css_media": metadata.get("document_css_media"),
        "document_css_required": metadata.get("document_css_required"),
        "css_link_href": emitted.get("css_link_href"),
        "css_link_media": emitted.get("css_link_media"),
        "css_link_injected": emitted.get("css_link_injected"),
        "css_link_preexisting": emitted.get("css_link_preexisting"),
    }
    RUN_REPORT_PATH.write_text(json.dumps(run_report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote {PDF_PATH} ({pdf_bytes} bytes)")
    print(f"[ok] HTML artifact: {HTML_PATH} ({html_css_emit_status})")
    print(f"[ok] CSS artifact: {CSS_PATH} ({html_css_emit_status})")
    print(f"[ok] Run report: {RUN_REPORT_PATH}")


if __name__ == "__main__":
    main()
