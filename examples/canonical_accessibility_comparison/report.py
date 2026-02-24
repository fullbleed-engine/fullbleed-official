from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import fullbleed

from fullbleed.ui import (
    Card,
    LayoutGrid,
    TBody,
    THead,
    Table,
    Td,
    Text,
    Th,
    Tr,
    el,
    validate_component_mount,
)
from fullbleed.ui.accessibility import (
    A11yContract,
    Alert,
    ColumnHeader,
    DataCell,
    Decorative,
    FigCaption,
    Figure,
    FieldGrid,
    FieldItem,
    Region,
    RowHeader,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
    SignatureBlock,
    Status,
)
from fullbleed.ui.core import Document


ROOT = Path(__file__).resolve().parent
OUTPUT_DIR = ROOT / "output"

NORMAL_HTML_PATH = OUTPUT_DIR / "canonical_normal.html"
NORMAL_PDF_PATH = OUTPUT_DIR / "canonical_normal.pdf"
NORMAL_A11Y_REPORT_PATH = OUTPUT_DIR / "canonical_normal_a11y_validation.json"
NORMAL_COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "canonical_normal_component_mount_validation.json"

A11Y_HTML_PATH = OUTPUT_DIR / "canonical_accessible.html"
A11Y_PDF_PATH = OUTPUT_DIR / "canonical_accessible.pdf"
A11Y_A11Y_REPORT_PATH = OUTPUT_DIR / "canonical_accessible_a11y_validation.json"
A11Y_COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "canonical_accessible_component_mount_validation.json"

COMPARISON_REPORT_PATH = OUTPUT_DIR / "canonical_comparison_report.json"

NORMAL_PNG_STEM = "canonical_normal"
A11Y_PNG_STEM = "canonical_accessible"


CSS = """
@page { size: letter; margin: 0.5in; }
body { margin: 0; font-family: Helvetica; font-size: 11pt; color: #172233; }
.comparison-root { display: block; }
.ui-main { display: block; }
.ui-card { border: 1px solid #d7e0ec; border-radius: 4px; padding: 10px; margin-bottom: 0.14in; }
.ui-section { margin-bottom: 0.16in; }
.ui-heading { margin: 0 0 6px; font-weight: 700; }
.ui-region { border: 1px solid #d7e0ec; border-radius: 4px; padding: 8px; margin-bottom: 10px; }
.ui-layout-grid { display: block; }
.normal-kv { display: block; margin: 0 0 4px; }
.normal-kv-label { font-weight: 700; display: inline-block; min-width: 1.45in; }
.normal-kv-value { display: inline; }
.signature-box { border: 1px dashed #c8d2e0; padding: 8px; margin-top: 6px; }
.sig-note { color: #4a607b; font-size: 10pt; margin-top: 4px; }
.ui-table, .ui-semantic-table { width: 100%; border-collapse: collapse; }
.ui-th, .ui-td, .ui-col-header, .ui-row-header, .ui-data-cell { border: 1px solid #a6b6cb; padding: 6px; }
.ui-th, .ui-col-header { background: #eef4fc; text-align: left; }
.ui-row-header { background: #f8fbff; text-align: left; }
.ui-table-caption { caption-side: top; text-align: left; font-weight: 700; margin-bottom: 6px; }
.ui-status { background: #edf8ee; border: 1px solid #bedfc2; padding: 6px; }
.ui-alert { background: #fff4e8; border: 1px solid #efcca1; padding: 6px; margin-top: 6px; }
.ui-field-grid { margin: 0; }
.ui-field-grid .ui-dt { font-weight: 700; margin-top: 2px; }
.ui-field-grid .ui-dd { margin: 0 0 4px 0; }
.ui-signature-block { border: 1px solid #d7e0ec; border-radius: 4px; padding: 8px; }
.ui-signature-mark { display: block; margin-top: 6px; width: 2.45in; height: 0.8in; }
.ui-figure { margin: 6px 0 0; }
.ui-figcaption { color: #4a607b; font-size: 10pt; }
""".strip()


@dataclass(frozen=True)
class TransactionRow:
    date: str
    description: str
    amount: str
    balance: str


@dataclass(frozen=True)
class StatementRecord:
    account_id: str
    period: str
    owner: str
    signature_status: str
    signer_name: str
    signed_at: str
    signature_method: str
    reference_id: str
    transactions: tuple[TransactionRow, ...]


DATA = StatementRecord(
    account_id="A-1042",
    period="2026-02-01 to 2026-02-29",
    owner="Jane Doe",
    signature_status="captured",
    signer_name="Jane Doe",
    signed_at="2026-02-23T11:42:00Z",
    signature_method="drawn_electronic",
    reference_id="audit-42f7",
    transactions=(
        TransactionRow("2026-02-10", "Invoice payment", "$500.00", "$1,240.00"),
        TransactionRow("2026-02-14", "Service charge", "-$12.00", "$1,228.00"),
        TransactionRow("2026-02-20", "Adjustment", "$40.00", "$1,268.00"),
    ),
)


def create_engine(*, document_title: str) -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        pdf_profile="tagged",
        document_lang="en-US",
        document_title=document_title,
    )


def _emit_preview_png(engine: fullbleed.PdfEngine, html: str, css: str, *, stem: str) -> list[str]:
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(OUTPUT_DIR), 144, stem) or [])
    if hasattr(engine, "render_image_pages"):
        page_images = list(engine.render_image_pages(html, css, 144) or [])
        out_paths: list[str] = []
        for idx, image_bytes in enumerate(page_images, start=1):
            path = OUTPUT_DIR / f"{stem}_page{idx}.png"
            path.write_bytes(image_bytes)
            out_paths.append(str(path))
        return out_paths
    return []


def _signature_svg() -> object:
    return el(
        "svg",
        el(
            "path",
            d="M6 46 C 40 12, 88 76, 136 38 C 160 20, 184 20, 208 34",
            fill="none",
            stroke="#183b73",
            stroke_width="4",
            stroke_linecap="round",
        ),
        el(
            "path",
            d="M142 50 C 164 58, 192 60, 214 44",
            fill="none",
            stroke="#183b73",
            stroke_width="3",
            stroke_linecap="round",
        ),
        viewBox="0 0 220 80",
        width="220",
        height="80",
    )


def _verification_seal_svg() -> object:
    return el(
        "svg",
        el("circle", cx="16", cy="16", r="14", fill="none", stroke="#c4cdd8", stroke_width="1.5"),
        el("path", d="M8 16 L13 21 L24 10", fill="none", stroke="#c4cdd8", stroke_width="2"),
        viewBox="0 0 32 32",
        width="24",
        height="24",
    )


@Document(page="LETTER", margin="0.5in", title="Canonical Example (Normal)", bootstrap=False)
def AppNormal(_props=None) -> object:
    return el(
        "div",
        Card(
            Text("Canonical Example: Normal Authoring", tag="h1"),
            Text(
                "Visually correct document built with generic primitives. Semantics remain partial and some accessibility signals are missing.",
                tag="p",
            ),
        ),
        Card(
            Text("Account Summary", tag="h2"),
            el(
                "div",
                el(
                    "div",
                    el("span", "Account", class_name="normal-kv-label"),
                    el("span", DATA.account_id, class_name="normal-kv-value"),
                    class_name="normal-kv",
                ),
                el(
                    "div",
                    el("span", "Statement period", class_name="normal-kv-label"),
                    el("span", DATA.period, class_name="normal-kv-value"),
                    class_name="normal-kv",
                ),
                el(
                    "div",
                    el("span", "Owner", class_name="normal-kv-label"),
                    el("span", DATA.owner, class_name="normal-kv-value"),
                    class_name="normal-kv",
                ),
            ),
        ),
        Card(
            Text("Transactions", tag="h2"),
            Table(
                THead(
                    Tr(
                        Th("Date"),
                        Th("Description"),
                        Th("Amount"),
                        Th("Balance"),
                    ),
                ),
                TBody(
                    *[
                        Tr(
                            Td(row.date),
                            Td(row.description),
                            Td(row.amount),
                            Td(row.balance),
                        )
                        for row in DATA.transactions
                    ]
                ),
            ),
        ),
        Card(
            Text("Signature", tag="h2"),
            el("div", f"Status: {DATA.signature_status.replace('_', ' ').title()}"),
            el("div", f"Signer: {DATA.signer_name}"),
            el("div", f"Timestamp: {DATA.signed_at}"),
            el("div", f"Method: {DATA.signature_method.replace('_', ' ')}"),
            el(
                "div",
                _signature_svg(),
                el(
                    "div",
                    "Visual signature mark for operator review only (normal variant uses no signature semantic wrapper).",
                    class_name="sig-note",
                ),
                class_name="signature-box",
            ),
            el(
                "div",
                _verification_seal_svg(),
                el("div", "Seal icon shown visually only.", class_name="sig-note"),
                class_name="signature-box",
            ),
        ),
        class_name="comparison-root",
    )


@Document(page="LETTER", margin="0.5in", title="Canonical Example (Accessible)", bootstrap=False)
def AppAccessible(_props=None) -> object:
    summary_heading_id = "summary-heading"
    tx_heading_id = "transactions-heading"
    sig_heading_id = "signature-heading"

    return el(
        "div",
        Section(
            Text(
                "Accessibility-first version using semantic field/value pairs, semantic table wrappers, labeled regions, and text-first signature semantics.",
                tag="p",
            ),
            heading="Canonical Example: Accessibility-First Authoring",
            heading_level=1,
        ),
        LayoutGrid(
            Region(
                Text("Account Summary", tag="h2", id=summary_heading_id),
                FieldGrid(
                    FieldItem("Account", DATA.account_id),
                    FieldItem("Statement period", DATA.period),
                    FieldItem("Owner", DATA.owner),
                ),
                labelledby=summary_heading_id,
            ),
            Region(
                Text("Transactions", tag="h2", id=tx_heading_id),
                SemanticTable(
                    SemanticTableHead(
                        SemanticTableRow(
                            ColumnHeader("Date"),
                            ColumnHeader("Description"),
                            ColumnHeader("Amount"),
                            ColumnHeader("Balance"),
                        )
                    ),
                    SemanticTableBody(
                        *[
                            SemanticTableRow(
                                RowHeader(row.date),
                                DataCell(row.description),
                                DataCell(row.amount),
                                DataCell(row.balance),
                            )
                            for row in DATA.transactions
                        ]
                    ),
                    caption="Transaction table",
                ),
                labelledby=tx_heading_id,
            ),
            Region(
                Text("Signature Evidence", tag="h2", id=sig_heading_id),
                Status("Signature capture completed and recorded."),
                Alert("Example alert: audit verification pending final review."),
                SignatureBlock(
                    signature_status=DATA.signature_status,
                    signer_name=DATA.signer_name,
                    timestamp=DATA.signed_at,
                    signature_method=DATA.signature_method,
                    reference_id=DATA.reference_id,
                    mark_node=_signature_svg(),
                    mark_decorative=False,
                ),
                Figure(
                    Decorative(_verification_seal_svg()),
                    FigCaption("Decorative verification seal. No additional signing meaning beyond the text above."),
                ),
                labelledby=sig_heading_id,
            ),
        ),
        class_name="comparison-root",
    )


def _artifact_title(artifact: object) -> str:
    title = getattr(artifact, "title", None)
    if isinstance(title, str) and title.strip():
        return title.strip()
    return "FullBleed Canonical Accessibility Comparison"


def _run_variant(
    *,
    variant_name: str,
    app: Callable[..., object],
    html_path: Path,
    pdf_path: Path,
    png_stem: str,
    component_validation_path: Path,
    a11y_validation_path: Path,
    strict_a11y_html: bool,
) -> dict[str, object]:
    artifact = app()  # type: ignore[misc]
    a11y_report = A11yContract().validate(artifact, mode=None)
    a11y_validation_path.write_text(json.dumps(a11y_report, indent=2), encoding="utf-8")

    html = artifact.to_html(a11y_mode="raise" if strict_a11y_html else None)
    html_path.write_text(html, encoding="utf-8")

    engine = create_engine(document_title=_artifact_title(artifact))
    component_validation = validate_component_mount(
        engine=engine,
        node_or_component=app,
        css=CSS,
        fail_on_overflow=True,
        fail_on_css_warnings=False,
        fail_on_known_loss=False,
        fail_on_html_asset_warning=True,
    )
    component_validation_path.write_text(
        json.dumps(component_validation, indent=2),
        encoding="utf-8",
    )

    bytes_written = int(engine.render_pdf_to_file(html, CSS, str(pdf_path)))
    png_paths = _emit_preview_png(engine, html, CSS, stem=png_stem)

    errors = list(a11y_report.get("errors") or [])
    warnings_only = list(a11y_report.get("warnings") or [])
    return {
        "variant": variant_name,
        "html_path": str(html_path),
        "pdf_path": str(pdf_path),
        "pdf_bytes": bytes_written,
        "png_paths": png_paths,
        "component_validation_path": str(component_validation_path),
        "component_validation_ok": bool(component_validation.get("ok", False)),
        "a11y_validation_path": str(a11y_validation_path),
        "a11y_ok": bool(a11y_report.get("ok", False)),
        "a11y_error_count": len(errors),
        "a11y_warning_count": len(warnings_only),
        "a11y_error_codes": [diag.get("code") for diag in errors],
        "a11y_warning_codes": [diag.get("code") for diag in warnings_only],
    }


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    normal = _run_variant(
        variant_name="normal",
        app=AppNormal,
        html_path=NORMAL_HTML_PATH,
        pdf_path=NORMAL_PDF_PATH,
        png_stem=NORMAL_PNG_STEM,
        component_validation_path=NORMAL_COMPONENT_VALIDATION_PATH,
        a11y_validation_path=NORMAL_A11Y_REPORT_PATH,
        strict_a11y_html=False,
    )
    accessible = _run_variant(
        variant_name="accessible",
        app=AppAccessible,
        html_path=A11Y_HTML_PATH,
        pdf_path=A11Y_PDF_PATH,
        png_stem=A11Y_PNG_STEM,
        component_validation_path=A11Y_COMPONENT_VALIDATION_PATH,
        a11y_validation_path=A11Y_A11Y_REPORT_PATH,
        strict_a11y_html=True,
    )

    report = {
        "schema": "fullbleed.canonical_accessibility_comparison.v1",
        "ok": (
            bool(normal["component_validation_ok"])
            and bool(accessible["component_validation_ok"])
            and bool(accessible["a11y_ok"])
        ),
        "normal": normal,
        "accessible": accessible,
        "delta": {
            "a11y_error_count_delta": int(normal["a11y_error_count"])
            - int(accessible["a11y_error_count"]),
            "a11y_warning_count_delta": int(normal["a11y_warning_count"])
            - int(accessible["a11y_warning_count"]),
        },
    }
    COMPARISON_REPORT_PATH.write_text(json.dumps(report, indent=2), encoding="utf-8")

    print(f"[ok] Normal PDF: {NORMAL_PDF_PATH} ({normal['pdf_bytes']} bytes)")
    print(f"[ok] Accessible PDF: {A11Y_PDF_PATH} ({accessible['pdf_bytes']} bytes)")
    print(f"[ok] Normal a11y: {NORMAL_A11Y_REPORT_PATH} (ok={normal['a11y_ok']})")
    print(f"[ok] Accessible a11y: {A11Y_A11Y_REPORT_PATH} (ok={accessible['a11y_ok']})")
    print(f"[ok] Comparison report: {COMPARISON_REPORT_PATH}")


if __name__ == "__main__":
    main()
