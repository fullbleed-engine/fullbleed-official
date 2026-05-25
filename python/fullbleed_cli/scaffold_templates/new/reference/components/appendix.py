from __future__ import annotations

from typing import Any

from .fb_ui import el
from .primitives import Badge, DataTable, KeyValueList, Page, PageHeader, Panel


def _coverage_cards(coverage: dict[str, Any]) -> object:
    summary = coverage.get("summary", {})
    cards = [
        ("Modules", summary.get("modules_total", "n/a")),
        ("Fixtures", summary.get("fixtures_label", "n/a")),
        ("Failed", summary.get("fixtures_failed", "n/a")),
        ("Source", summary.get("source", "fallback")),
    ]
    return el(
        "section",
        [
            el(
                "article",
                el("p", label, class_name="ref-coverage-label"),
                el("p", value, class_name="ref-coverage-value"),
                class_name="ref-coverage-card",
            )
            for label, value in cards
        ],
        class_name="ref-coverage-cards",
    )


def AppendixPages(data: dict[str, Any], coverage: dict[str, Any]) -> list[object]:
    module_rows = coverage.get("modules", [])[:5]
    if not module_rows:
        module_rows = [
            {
                "label": "No local coverage report",
                "status": "n/a",
                "priority": "n/a",
                "notes": "Run the CSS fixture suite to populate _css_working/css_parity_status.json.",
            }
        ]
    module_columns = [
        ("label", "CSS module"),
        ("status", "Status"),
        ("priority", "Priority"),
        ("notes", "Notes"),
    ]
    limit_rows = [
        {
            "topic": item["topic"],
            "detail": item["detail"],
        }
        for item in data["known_limits"]
    ]
    feature_rows = [
        {
            **row,
            "status": Badge(row["status"], tone="success" if row["status"] == "Covered" else "neutral"),
        }
        for row in data["feature_matrix"]
    ]
    return [
        Page(
            PageHeader(
                "Appendix",
                "Coverage snapshot and operating contract",
                "Runtime artifacts make the example auditable instead of merely illustrative.",
            ),
            _coverage_cards(coverage),
            el(
                "section",
                Panel(
                    DataTable(
                        module_columns,
                        module_rows,
                        caption="Local CSS coverage snapshot",
                        class_name="ref-table-compact",
                    ),
                    title="CSS readiness",
                    class_name="ref-panel-table",
                ),
                Panel(
                    KeyValueList(
                        [
                            ("Strict validation", "FULLBLEED_VALIDATE_STRICT=1"),
                            ("Page data", "paginated_context ops"),
                            ("Preview DPI", "FULLBLEED_IMAGE_DPI=144"),
                            ("JIT trace", "FULLBLEED_DEBUG=1"),
                            ("Performance trace", "FULLBLEED_PERF=1"),
                            ("Footers", "footer_first, footer_each, footer_last"),
                        ]
                    ),
                    title="Runtime switches",
                ),
                class_name="ref-two-column",
            ),
            Panel(
                DataTable(
                    [("topic", "Limit"), ("detail", "Static PDF policy")],
                    limit_rows,
                    caption="Known static-output limits",
                    class_name="ref-table-compact",
                ),
                title="Limits",
                class_name="ref-panel-table",
            ),
            class_name="ref-appendix-page",
            page_id="appendix-coverage",
        ),
        Page(
            PageHeader(
                "Appendix",
                "Full feature evidence",
                "The long evidence matrix is isolated on its own authored page so it does not depend on overflow splitting.",
            ),
            Panel(
                DataTable(
                    [("area", "Area"), ("feature", "Feature"), ("status", "Status"), ("evidence", "Evidence")],
                    feature_rows,
                    caption="Full feature evidence",
                    class_name="ref-table-compact ref-table-small",
                ),
                title="Evidence",
                class_name="ref-panel-table ref-panel-wide",
            ),
            class_name="ref-appendix-page ref-appendix-evidence-page",
            page_id="appendix-evidence",
        ),
    ]
