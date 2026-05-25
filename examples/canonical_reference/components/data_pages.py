from __future__ import annotations

from typing import Any

from .fb_ui import el
from .primitives import Badge, DataTable, Page, PageHeader, Panel


def _status_cell(row: dict[str, Any]) -> dict[str, Any]:
    enriched = dict(row)
    status = str(row.get("status", ""))
    tone = "success" if status.lower() == "ready" else "neutral"
    enriched["status"] = Badge(status, tone=tone)
    return enriched


def DataPages(data: dict[str, Any]) -> list[object]:
    account_columns = [
        ("id", "ID"),
        ("name", "Reference surface"),
        ("owner", "Owner"),
        ("status", "Status"),
        ("risk", "Risk"),
        ("last_review", "Review"),
    ]
    transaction_columns = [
        ("date", "Date"),
        ("description", "Event"),
        ("category", "Category"),
        ("amount", "Weight"),
        ("balance", "Balance"),
    ]
    feature_columns = [
        ("area", "Area"),
        ("feature", "Feature"),
        ("status", "Status"),
        ("evidence", "Evidence"),
    ]
    account_rows = [_status_cell(row) for row in data["accounts"]]
    transaction_rows = [
        {
            **row,
            "_data_fb": (
                f"reference.weight={row['amount']}; "
                f"reference.event={row['date']}; "
                f"reference.category={row['category']}"
            ),
        }
        for row in data["transactions"]
    ]
    feature_rows = [
        {
            **row,
            "status": Badge(row["status"], tone="success" if row["status"] == "Covered" else "neutral"),
        }
        for row in data["feature_matrix"][:6]
    ]
    return [
        Page(
            PageHeader(
                "Data",
                "Tables and report records",
                "The example uses ordinary JSON input and renders accessible table structures.",
            ),
            Panel(
                DataTable(account_columns, account_rows, caption="Reference surface inventory", class_name="ref-table-compact"),
                title="Inventory table",
                class_name="ref-panel-table",
            ),
            Panel(
                DataTable(transaction_columns, transaction_rows, caption="Reference event ledger", class_name="ref-table-compact"),
                title="Ledger table",
                class_name="ref-panel-table",
            ),
            class_name="ref-data-page",
            page_id="data-tables",
        ),
        Page(
            PageHeader(
                "Coverage",
                "Feature matrix",
                "The matrix is concise enough for business PDFs and still exposes concrete evidence.",
            ),
            Panel(
                DataTable(feature_columns, feature_rows, caption="Canonical feature evidence", class_name="ref-table-compact"),
                title="Evidence table",
                class_name="ref-panel-table",
            ),
            el(
                "aside",
                el("p", "The full matrix is repeated in the appendix with runtime coverage data when local CSS reports are available."),
                class_name="ref-note",
            ),
            class_name="ref-feature-page",
            page_id="feature-matrix",
        ),
    ]
