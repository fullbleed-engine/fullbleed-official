import csv
import json
import os
import re
import tempfile
from dataclasses import dataclass
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path

import fullbleed

from components.fb_ui import Document, compile_document, el, validate_component_mount
from components.header import Header
from components.body import Body
from components.footer import Footer

ROOT = Path(__file__).resolve().parent
DATA_PATH = ROOT / "data" / "invoice.csv"
OUTPUT_DIR = ROOT / "output"
PDF_PATH = OUTPUT_DIR / "acme_sample_invoice.pdf"
PREVIEW_PNG_STEM = "acme_sample_invoice"
PAGE_DATA_PATH = OUTPUT_DIR / "acme_sample_invoice_page_data.json"
JIT_PATH = OUTPUT_DIR / "acme_sample_invoice.jit.jsonl"
PERF_PATH = OUTPUT_DIR / "acme_sample_invoice.perf.jsonl"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "acme_sample_invoice_component_mount_validation.json"
CSS_LAYER_REPORT_PATH = OUTPUT_DIR / "acme_sample_invoice_css_layers.json"

CSS_LAYER_ORDER = [
    "styles/tokens.css",
    "components/styles/primitives.css",
    "components/styles/header.css",
    "components/styles/body.css",
    "components/styles/footer.css",
    "styles/report.css",
]

# Mirrors parser signals in `src/style.rs` where these declarations are parsed
# but currently treated as known no-effect loss points.
NO_EFFECT_PROPERTIES = {
    "align-content",
    "align-self",
    "justify-items",
    "justify-self",
    "place-content",
    "place-items",
    "place-self",
    "row-gap",
    "column-gap",
    "flex-flow",
    "grid-template-rows",
    "grid-auto-columns",
    "grid-auto-rows",
    "grid-auto-flow",
    "grid-template-areas",
    "grid-template",
    "grid",
    "grid-row-start",
    "grid-row-end",
    "grid-column-start",
    "grid-column-end",
    "grid-row",
    "grid-column",
    "grid-area",
}

NORMALIZED_DISPLAY_VALUES = {
    "table-column",
    "table-column-group",
    "ruby",
    "ruby-base",
    "ruby-text",
    "ruby-base-container",
    "ruby-text-container",
}


@dataclass(frozen=True)
class InvoiceItem:
    description: str
    qty: Decimal
    rate: Decimal
    amount: Decimal


@dataclass(frozen=True)
class InvoiceData:
    meta: dict[str, str]
    items: list[InvoiceItem]
    subtotal: Decimal
    tax_rate: Decimal
    tax_amount: Decimal
    total: Decimal


def _env_truthy(name: str) -> bool:
    value = os.getenv(name, "").strip().lower()
    return value in {"1", "true", "yes", "on"}


def _env_int(name: str, default: int) -> int:
    raw = os.getenv(name, "").strip()
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        return default


def _money(value: Decimal | str) -> Decimal:
    return Decimal(str(value)).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


def _fmt_money(value: Decimal) -> str:
    return f"${value:.2f}"


def _fmt_qty(value: Decimal) -> str:
    if value == value.to_integral_value():
        return str(int(value))
    return f"{value.normalize()}"


def _fmt_percent(value: Decimal) -> str:
    pct = (value * Decimal("100")).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)
    if pct == pct.to_integral_value():
        return str(int(pct))
    return f"{pct:.2f}".rstrip("0").rstrip(".")


def _required(meta: dict[str, str], key: str) -> str:
    value = (meta.get(key) or "").strip()
    if not value:
        raise ValueError(f"Missing required meta field in CSV: {key}")
    return value


def load_invoice_data(path: Path = DATA_PATH) -> InvoiceData:
    meta: dict[str, str] = {}
    items: list[InvoiceItem] = []

    with path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            row_type = (row.get("row_type") or "").strip().lower()
            if row_type == "meta":
                key = (row.get("key") or "").strip()
                value = (row.get("value") or "").strip()
                if key:
                    meta[key] = value
                continue
            if row_type == "item":
                description = (row.get("description") or "").strip()
                qty_text = (row.get("qty") or "").strip()
                rate_text = (row.get("rate") or "").strip()
                if not description:
                    raise ValueError("CSV item row is missing description")
                if not qty_text or not rate_text:
                    raise ValueError(f"CSV item row missing qty/rate for: {description}")

                qty = Decimal(qty_text)
                rate = _money(rate_text)
                amount = _money(qty * rate)
                items.append(
                    InvoiceItem(
                        description=description,
                        qty=qty,
                        rate=rate,
                        amount=amount,
                    )
                )

    if not items:
        raise ValueError("CSV must include at least one item row")

    required_meta = [
        "studio_name",
        "studio_tagline",
        "from_company",
        "from_email",
        "from_website",
        "from_address_line1",
        "from_address_line2",
        "bill_company",
        "bill_contact",
        "bill_email",
        "bill_address_line1",
        "bill_address_line2",
        "invoice_number",
        "invoice_date",
    ]
    for key in required_meta:
        _required(meta, key)

    tax_rate = Decimal(meta.get("tax_rate", "0.08"))
    subtotal = _money(sum((item.amount for item in items), Decimal("0.00")))
    tax_amount = _money(subtotal * tax_rate)
    total = _money(subtotal + tax_amount)
    return InvoiceData(
        meta=meta,
        items=items,
        subtotal=subtotal,
        tax_rate=tax_rate,
        tax_amount=tax_amount,
        total=total,
    )


def create_engine(*, debug: bool | None = None, debug_out: str | None = None, jit_mode: str | None = None):
    """Build the rendering engine and register asset bundles.

    Important:
    - Engine page geometry (page width/height/margin) is configured here.
    - `@Document(...)` in component code is metadata and authoring structure, not engine config.
    """
    bundle = fullbleed.AssetBundle()

    # Vendored defaults from `fullbleed init`.
    bundle.add_file(str(ROOT / "vendor/css/bootstrap.min.css"), "css", name="bootstrap")
    bundle.add_file(str(ROOT / "vendor/fonts/Inter-Variable.ttf"), "font")
    bundle.add_file(str(ROOT / "vendor/icons/bootstrap-icons.svg"), "svg", name="bootstrap-icons")

    debug_enabled = _env_truthy("FULLBLEED_DEBUG") if debug is None else bool(debug)
    debug_target = debug_out if debug_out is not None else (str(JIT_PATH) if debug_enabled else None)

    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        debug=debug_enabled,
        debug_out=debug_target,
        perf=_env_truthy("FULLBLEED_PERF"),
        perf_out=str(PERF_PATH) if _env_truthy("FULLBLEED_PERF") else None,
        jit_mode=jit_mode,
    )

    engine.register_bundle(bundle)
    return engine


@Document(page="LETTER", margin="0in", title="Acme Sample Invoice", bootstrap=False)
def App(_props=None):
    invoice = load_invoice_data(DATA_PATH)
    items = [
        {
            "description": item.description,
            "qty": _fmt_qty(item.qty),
            "rate": _fmt_money(item.rate),
            "amount": _fmt_money(item.amount),
        }
        for item in invoice.items
    ]
    tax_label = f"Tax ({_fmt_percent(invoice.tax_rate)}%)"
    return [
        el(
            "section",
            Header(invoice=invoice.meta),
            Body(items=items),
            Footer(
                subtotal=_fmt_money(invoice.subtotal),
                tax_label=tax_label,
                tax_amount=_fmt_money(invoice.tax_amount),
                total=_fmt_money(invoice.total),
            ),
            class_name="fb-invoice-page",
        )
    ]


def build_html():
    artifact = App()
    return compile_document(artifact)


def _selector_scope_ok(selector: str) -> bool:
    cleaned = selector.strip()
    if not cleaned:
        return True
    if cleaned.startswith("@"):
        return True
    if cleaned in {":root", "html", "body"}:
        return True
    if cleaned.startswith("html ") or cleaned.startswith("body "):
        return True
    if '[data-fb-role="document-root"]' in cleaned:
        return True
    if "[data-fb-role='document-root']" in cleaned:
        return True
    if ".fb-document-root" in cleaned:
        return True
    return False


def _find_unscoped_selectors(css_text: str) -> list[str]:
    findings: list[str] = []
    for raw in re.findall(r"([^{}]+)\{", css_text):
        header = raw.strip()
        if not header:
            continue
        # Skip nested at-rule headers (media/supports/etc.).
        if header.startswith("@"):
            continue
        for selector in [part.strip() for part in header.split(",")]:
            if not selector:
                continue
            if _selector_scope_ok(selector):
                continue
            findings.append(selector)
            if len(findings) >= 20:
                return findings
    return findings


def _find_engine_no_effect_declarations(css_text: str) -> list[dict[str, str]]:
    findings: list[dict[str, str]] = []
    for match in re.finditer(r"([a-zA-Z-]+)\s*:\s*([^;{}]+)", css_text):
        prop = match.group(1).strip().lower()
        value = match.group(2).strip().lower()
        if prop in NO_EFFECT_PROPERTIES:
            findings.append({"property": prop, "value": value})
        elif prop == "display" and any(token in value for token in NORMALIZED_DISPLAY_VALUES):
            findings.append({"property": prop, "value": value})
        if len(findings) >= 20:
            break
    return findings


def load_css_layers():
    manifest: list[dict[str, object]] = []
    css_parts: list[str] = []
    unscoped: list[dict[str, str]] = []
    no_effect: list[dict[str, str]] = []

    for rel in CSS_LAYER_ORDER:
        path = ROOT / rel
        exists = path.exists()
        text = path.read_text(encoding="utf-8") if exists else ""
        byte_count = len(text.encode("utf-8")) if exists else 0
        manifest.append({"path": rel, "exists": exists, "bytes": byte_count})
        if not exists or not text.strip():
            continue
        css_parts.append(f"/* layer: {rel} */\n{text}")

        if rel.startswith("components/styles/"):
            for selector in _find_unscoped_selectors(text):
                unscoped.append({"layer": rel, "selector": selector})
            for finding in _find_engine_no_effect_declarations(text):
                no_effect.append({"layer": rel, **finding})

    return "\n\n".join(css_parts), manifest, unscoped, no_effect


def _emit_preview_png(engine, html: str, css: str, out_dir: Path, *, stem: str, dpi: int) -> tuple[str, str | None]:
    if hasattr(engine, "render_image_pages_to_dir"):
        paths = engine.render_image_pages_to_dir(html, css, str(out_dir), dpi, stem)
        if paths:
            return "ok", str(paths[0])
        return "skipped (no pages)", None

    if hasattr(engine, "render_image_pages"):
        page_images = engine.render_image_pages(html, css, dpi)
        if page_images:
            first_path = out_dir / f"{stem}_page1.png"
            first_path.write_bytes(page_images[0])
            return "ok", str(first_path)
        return "skipped (no pages)", None

    return "skipped (engine image API unavailable)", None


def main():
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    strict_validate = _env_truthy("FULLBLEED_VALIDATE_STRICT")
    html = build_html()
    css, css_layers, unscoped_selectors, no_effect_declarations = load_css_layers()
    CSS_LAYER_REPORT_PATH.write_text(
        json.dumps(
            {
                "layers": css_layers,
                "unscoped_selector_count": len(unscoped_selectors),
                "no_effect_declaration_count": len(no_effect_declarations),
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    emit_page_data = _env_truthy("FULLBLEED_EMIT_PAGE_DATA")
    image_dpi = _env_int("FULLBLEED_IMAGE_DPI", 144)

    if unscoped_selectors:
        print(f"[warn] Found {len(unscoped_selectors)} unscoped selector(s) in component CSS.")
        for item in unscoped_selectors[:5]:
            print(f"[warn] {item['layer']}: {item['selector']}")
        if strict_validate:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and unscoped selectors were found.")
            raise SystemExit(2)
    if no_effect_declarations:
        print(f"[warn] Found {len(no_effect_declarations)} engine no-effect declaration(s) in component CSS.")
        for item in no_effect_declarations[:5]:
            print(f"[warn] {item['layer']}: {item['property']}: {item['value']}")
        if strict_validate:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and engine no-effect declarations were found.")
            raise SystemExit(2)

    with tempfile.NamedTemporaryFile(prefix="invoice_mount_", suffix=".jit.jsonl", delete=False) as tmp:
        mount_jit_path = Path(tmp.name)

    try:
        validation_engine = create_engine(debug=True, debug_out=str(mount_jit_path), jit_mode="plan")
        mount_validation = validate_component_mount(
            engine=validation_engine,
            node_or_component=App,
            css=css,
            debug_log=str(mount_jit_path),
            title="component mount smoke",
            fail_on_overflow=False,
            fail_on_css_warnings=False,
            fail_on_known_loss=strict_validate,
            fail_on_html_asset_warning=True,
        )
    finally:
        if mount_jit_path.exists():
            mount_jit_path.unlink(missing_ok=True)

    COMPONENT_VALIDATION_PATH.write_text(json.dumps(mount_validation, indent=2), encoding="utf-8")
    if not mount_validation.get("ok", False):
        print(f"[error] Component mount validation failed: {COMPONENT_VALIDATION_PATH}")
        raise SystemExit(2)
    validation_warnings = mount_validation.get("warnings") or []
    if validation_warnings:
        print(f"[warn] Component mount validation warnings: {len(validation_warnings)}")
        blocking_warnings = [
            warning
            for warning in validation_warnings
            if str(warning.get("code", "")).upper() != "OVERFLOW"
        ]
        if strict_validate and blocking_warnings:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and mount warnings were detected.")
            raise SystemExit(2)
    print(f"[ok] Component mount validation: {COMPONENT_VALIDATION_PATH}")

    engine = create_engine()

    if emit_page_data:
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        PDF_PATH.write_bytes(pdf_bytes)
        bytes_written = len(pdf_bytes)
        if page_data is not None:
            PAGE_DATA_PATH.write_text(json.dumps(page_data, indent=2), encoding="utf-8")
    else:
        bytes_written = engine.render_pdf_to_file(html, css, str(PDF_PATH))

    png_status, preview_png = _emit_preview_png(
        engine,
        html,
        css,
        OUTPUT_DIR,
        stem=PREVIEW_PNG_STEM,
        dpi=image_dpi,
    )

    print(f"[ok] CSV: {DATA_PATH}")
    print(f"[ok] Wrote {PDF_PATH} ({bytes_written} bytes)")
    if preview_png is not None:
        print(f"[ok] Preview PNG: {preview_png} ({png_status})")
    else:
        print(f"[ok] Preview PNG: {OUTPUT_DIR} ({png_status})")
    print(f"[ok] CSS layers: {CSS_LAYER_REPORT_PATH}")
    if emit_page_data:
        page_data_status = "ok" if PAGE_DATA_PATH.exists() else "skipped (engine returned none)"
        print(f"[ok] Page data: {PAGE_DATA_PATH} ({page_data_status})")
    if _env_truthy("FULLBLEED_DEBUG"):
        print(f"[ok] JIT trace: {JIT_PATH}")
    if _env_truthy("FULLBLEED_PERF"):
        print(f"[ok] Perf trace: {PERF_PATH}")


if __name__ == "__main__":
    main()
