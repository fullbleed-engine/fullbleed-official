import csv
import json
import os
import re
import tempfile
from dataclasses import dataclass
from datetime import datetime
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path

import fullbleed

from components.fb_ui import Document, compile_document, validate_component_mount
from components.body import Body
from components.header import Header

ROOT = Path(__file__).resolve().parent
DATA_PATH = ROOT / "data" / "statement.csv"
OUTPUT_DIR = ROOT / "output"
PDF_PATH = OUTPUT_DIR / "bank_statement.pdf"
PREVIEW_PNG_STEM = "bank_statement"
PAGE_DATA_PATH = OUTPUT_DIR / "bank_statement_page_data.json"
JIT_PATH = OUTPUT_DIR / "bank_statement.jit.jsonl"
PERF_PATH = OUTPUT_DIR / "bank_statement.perf.jsonl"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "bank_statement_component_mount_validation.json"
CSS_LAYER_REPORT_PATH = OUTPUT_DIR / "bank_statement_css_layers.json"

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
class Transaction:
    date_iso: str
    description: str
    amount: Decimal


@dataclass(frozen=True)
class StatementData:
    meta: dict[str, str]
    summary: list[dict[str, str]]
    transactions: list[dict[str, str]]


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


def _fmt_signed(value: Decimal) -> str:
    sign = "+" if value >= Decimal("0.00") else "-"
    return f"{sign}${abs(value):.2f}"


def _required(meta: dict[str, str], key: str) -> str:
    value = (meta.get(key) or "").strip()
    if not value:
        raise ValueError(f"Missing required meta field in CSV: {key}")
    return value


def _date_label(iso_date: str) -> str:
    parsed = datetime.strptime(iso_date, "%Y-%m-%d")
    return parsed.strftime("%b %d").replace(" 0", " ")


def load_statement_data(path: Path = DATA_PATH) -> StatementData:
    meta: dict[str, str] = {}
    tx_rows: list[Transaction] = []

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
            if row_type == "transaction":
                date_iso = (row.get("date") or "").strip()
                description = (row.get("description") or "").strip()
                amount_text = (row.get("amount") or "").strip()
                if not date_iso or not description or not amount_text:
                    raise ValueError(f"Incomplete transaction row: {row}")
                tx_rows.append(
                    Transaction(
                        date_iso=date_iso,
                        description=description,
                        amount=_money(amount_text),
                    )
                )

    required_meta = [
        "bank_name",
        "bank_tagline",
        "contact_phone",
        "contact_email",
        "contact_website",
        "account_holder",
        "account_address_line1",
        "account_address_line2",
        "account_number",
        "routing_number",
        "statement_start",
        "statement_end",
        "beginning_balance",
    ]
    for key in required_meta:
        _required(meta, key)
    if not tx_rows:
        raise ValueError("CSV must include at least one transaction row")

    begin_balance = _money(meta["beginning_balance"])
    ordered_tx = sorted(tx_rows, key=lambda item: item.date_iso)
    running = begin_balance
    balance_map: dict[tuple[str, str, str], Decimal] = {}
    total_deposits = Decimal("0.00")
    total_withdrawals = Decimal("0.00")

    for tx in ordered_tx:
        running = _money(running + tx.amount)
        if tx.amount >= Decimal("0.00"):
            total_deposits = _money(total_deposits + tx.amount)
        else:
            total_withdrawals = _money(total_withdrawals + abs(tx.amount))
        balance_map[(tx.date_iso, tx.description, f"{tx.amount:.2f}")] = running

    display_rows: list[dict[str, str]] = []
    for tx in reversed(ordered_tx):
        key = (tx.date_iso, tx.description, f"{tx.amount:.2f}")
        balance = balance_map[key]
        amount_class = "positive" if tx.amount >= Decimal("0.00") else "negative"
        display_rows.append(
            {
                "date": _date_label(tx.date_iso),
                "description": tx.description,
                "amount": _fmt_signed(tx.amount),
                "amount_raw": f"{tx.amount:.2f}",
                "amount_class": amount_class,
                "balance": _fmt_money(balance),
            }
        )

    ending_balance = _money(running)
    statement_period = f"{meta['statement_start']} - {meta['statement_end']}"
    shaped_meta = dict(meta)
    shaped_meta["statement_period"] = statement_period

    summary = [
        {
            "label": "Beginning Balance",
            "value": _fmt_money(begin_balance),
            "tone": "neutral",
        },
        {
            "label": "Total Deposits",
            "value": f"+${total_deposits:.2f}",
            "tone": "positive",
        },
        {
            "label": "Total Withdrawals",
            "value": f"-${total_withdrawals:.2f}",
            "tone": "negative",
        },
        {
            "label": "Ending Balance",
            "value": _fmt_money(ending_balance),
            "tone": "neutral",
        },
    ]

    return StatementData(
        meta=shaped_meta,
        summary=summary,
        transactions=display_rows,
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
        page_margins={
            1: {"top": "0in", "right": "0in", "bottom": "0in", "left": "0in"},
            2: {"top": "0.42in", "right": "0.5in", "bottom": "0.4in", "left": "0.5in"},
            "n": {"top": "0.42in", "right": "0.5in", "bottom": "0.4in", "left": "0.5in"},
        },
        header_each="Bank Statement Continued - Page {page} of {pages}",
        header_x="0.5in",
        header_y_from_top="0.16in",
        header_font_name="Inter",
        header_font_size=9.0,
        header_color="#5a6d84",
        paginated_context={"tx.amount": "sum"},
        footer_each="Page {page} of {pages}  |  Net Activity This Page: ${sum:tx.amount}",
        footer_last="Page {page} of {pages}  |  Net Activity Total: ${total:tx.amount}",
        footer_x="0.5in",
        footer_y_from_bottom="0.16in",
        debug=debug_enabled,
        debug_out=debug_target,
        perf=_env_truthy("FULLBLEED_PERF"),
        perf_out=str(PERF_PATH) if _env_truthy("FULLBLEED_PERF") else None,
        jit_mode=jit_mode,
    )

    engine.register_bundle(bundle)
    return engine


@Document(page="LETTER", margin="0in", title="Horizon Bank Statement", bootstrap=False)
def App(_props=None):
    statement = load_statement_data(DATA_PATH)
    return [
        Header(meta=statement.meta),
        Body(summary=statement.summary, transactions=statement.transactions),
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

    with tempfile.NamedTemporaryFile(prefix="bank_statement_mount_", suffix=".jit.jsonl", delete=False) as tmp:
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

