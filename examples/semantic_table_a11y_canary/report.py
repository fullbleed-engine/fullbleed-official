from __future__ import annotations

import json
from pathlib import Path

import fullbleed

from fullbleed.ui import LayoutGrid, el, validate_component_mount
from fullbleed.ui.accessibility import (
    A11yContract,
    A11yAttrs,
    FieldGrid,
    FieldItem,
    Region,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
    ColumnHeader,
    RowHeader,
    DataCell,
)
from fullbleed.ui.core import Document


ROOT = Path(__file__).resolve().parent
OUTPUT_DIR = ROOT / "output"
PDF_PATH = OUTPUT_DIR / "semantic_table_a11y_canary.pdf"
A11Y_VALIDATION_PATH = OUTPUT_DIR / "semantic_table_a11y_canary_a11y_validation.json"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "semantic_table_a11y_canary_component_mount_validation.json"
RUN_REPORT_PATH = OUTPUT_DIR / "semantic_table_a11y_canary_run_report.json"
PREVIEW_PNG_STEM = "semantic_table_a11y_canary"

CSS = """
@page { size: letter; margin: 0.5in; }
body { margin: 0; font-family: Helvetica; font-size: 11pt; color: #142033; }
.ui-main { display: block; }
.ui-section { margin-bottom: 0.18in; }
.ui-heading { margin: 0 0 0.08in; font-weight: 700; }
.ui-layout-grid { display: block; }
.ui-layout-grid .ui-region { margin-bottom: 0.12in; }
.ui-field-grid { margin: 0; }
.ui-field-grid .ui-dt { font-weight: 700; margin-top: 0.03in; }
.ui-field-grid .ui-dd { margin: 0 0 0.05in 0; }
.ui-semantic-table { width: 100%; border-collapse: collapse; }
.ui-semantic-table .ui-col-header,
.ui-semantic-table .ui-row-header,
.ui-semantic-table .ui-data-cell { border: 1px solid #a5b2c6; padding: 6px; }
.ui-semantic-table .ui-col-header { background: #edf3fb; text-align: left; }
.ui-semantic-table .ui-row-header { background: #f8fbff; text-align: left; }
.ui-table-caption { caption-side: top; text-align: left; font-weight: 700; margin-bottom: 6px; }
.ui-region { border: 1px solid #d8e1ee; padding: 8px; border-radius: 4px; }
""".strip()


def create_engine() -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        pdf_profile="tagged",
        document_lang="en-US",
        document_title="Semantic Table Accessibility Canary",
    )


def _emit_preview_png(engine: fullbleed.PdfEngine, html: str, css: str, out_dir: Path, *, stem: str) -> list[str]:
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
    if hasattr(engine, "render_image_pages"):
        page_images = list(engine.render_image_pages(html, css, 144) or [])
        paths: list[str] = []
        for idx, image_bytes in enumerate(page_images, start=1):
            path = out_dir / f"{stem}_page{idx}.png"
            path.write_bytes(image_bytes)
            paths.append(str(path))
        return paths
    return []


@Document(
    page="LETTER",
    margin="0.5in",
    title="Semantic Table Accessibility Canary",
    bootstrap=False,
)
def App(_props=None) -> object:
    summary_label_id = "record-summary-label"
    summary_help_id = "record-summary-help"
    table_region_id = "transactions-region"
    table_heading_id = "transactions-heading"

    summary_region = Region(
        FieldGrid(
            FieldItem("Account", "A-1042"),
            FieldItem("Statement period", "2026-02-01 to 2026-02-29"),
            FieldItem("Owner", "Jane Doe"),
        ),
        label="Record summary",
        **A11yAttrs.merge(
            A11yAttrs.id(table_region_id),
            A11yAttrs.describedby(summary_help_id),
        ),
        class_name="summary-region",
    )

    table_region = Region(
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
                SemanticTableRow(
                    RowHeader("2026-02-10"),
                    DataCell("Invoice payment"),
                    DataCell("$500.00"),
                    DataCell("$1,240.00"),
                ),
                SemanticTableRow(
                    RowHeader("2026-02-14"),
                    DataCell("Service charge"),
                    DataCell("-$12.00"),
                    DataCell("$1,228.00"),
                ),
                SemanticTableRow(
                    RowHeader("2026-02-20"),
                    DataCell("Adjustment"),
                    DataCell("$40.00"),
                    DataCell("$1,268.00"),
                ),
            ),
            caption="Transaction table",
        ),
        labelledby=table_heading_id,
        class_name="table-region",
    )

    return el(
        "div",
        Section(
            el(
                "p",
                "Canary example for SemanticTable, FieldGrid, Region labels, and tagged-PDF table scope.",
            ),
            heading="Accessibility + Semantic Table Canary",
            heading_level=1,
            class_name="intro",
        ),
        LayoutGrid(
            Region(
                el("h2", "Record Summary", id=summary_label_id, class_name="ui-heading"),
                el(
                    "p",
                    "Structured field/value pairs are emitted as dl/dt/dd and validated for reading order.",
                    id=summary_help_id,
                ),
                summary_region,
                label="Summary panel",
            ),
            Region(
                el("h2", "Transactions", id=table_heading_id, class_name="ui-heading"),
                table_region,
                label="Transactions panel",
            ),
        ),
        class_name="semantic-table-a11y-canary-root",
    )


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    engine = create_engine()
    artifact = App()
    a11y_report = A11yContract().validate(artifact, mode=None)
    A11Y_VALIDATION_PATH.write_text(json.dumps(a11y_report, indent=2), encoding="utf-8")

    html = artifact.to_html(a11y_mode="raise")
    component_validation = validate_component_mount(
        engine=engine,
        node_or_component=App,
        css=CSS,
        fail_on_overflow=True,
        fail_on_css_warnings=False,
        fail_on_known_loss=False,
        fail_on_html_asset_warning=True,
    )
    COMPONENT_VALIDATION_PATH.write_text(
        json.dumps(component_validation, indent=2),
        encoding="utf-8",
    )

    bytes_written = engine.render_pdf_to_file(html, CSS, str(PDF_PATH))
    png_paths = _emit_preview_png(engine, html, CSS, OUTPUT_DIR, stem=PREVIEW_PNG_STEM)

    run_report = {
        "schema": "fullbleed.a11y_canary.run.v1",
        "ok": bool(a11y_report.get("ok", False)) and bool(component_validation.get("ok", False)),
        "pdf_path": str(PDF_PATH),
        "pdf_bytes": int(bytes_written),
        "png_paths": png_paths,
        "a11y_validation_path": str(A11Y_VALIDATION_PATH),
        "component_validation_path": str(COMPONENT_VALIDATION_PATH),
    }
    RUN_REPORT_PATH.write_text(json.dumps(run_report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] A11y validation: {A11Y_VALIDATION_PATH} (ok={a11y_report.get('ok')})")
    print(
        f"[ok] Component validation: {COMPONENT_VALIDATION_PATH} (ok={component_validation.get('ok')})"
    )
    if png_paths:
        print(f"[ok] Preview PNGs: {len(png_paths)}")


if __name__ == "__main__":
    main()
