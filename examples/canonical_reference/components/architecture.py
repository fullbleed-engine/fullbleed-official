from __future__ import annotations

from typing import Any

from .fb_ui import el
from .primitives import DataTable, KeyValueList, Page, PageHeader, Panel, StepList


def ArchitecturePage(data: dict[str, Any]) -> object:
    scaffold_columns = [
        ("path", "Path"),
        ("purpose", "Purpose"),
    ]
    contract = [
        ("Engine geometry", "8.5in by 11in, margin 0in"),
        ("Authoring geometry", "Page sections define padding and page breaks in CSS"),
        ("CSS packaging", "Linked HTML artifact plus explicit CSS passed to the engine"),
        ("Asset policy", "Local or bundled assets only for deterministic static PDFs"),
        ("Strict mode", "FULLBLEED_VALIDATE_STRICT=1 promotes CSS and validation warnings to failures"),
    ]
    return Page(
        PageHeader(
            "Scaffold",
            "Canonical project shape",
            "The reference is deliberately organized like a downstream production report.",
        ),
        el(
            "section",
            Panel(
                StepList(data["pipeline"]),
                title="Render pipeline",
                class_name="ref-panel-pipeline",
            ),
            Panel(
                DataTable(scaffold_columns, data["scaffold"], caption="Scaffold inventory"),
                title="Folder contract",
            ),
            class_name="ref-two-column",
        ),
        el(
            "section",
            Panel(
                KeyValueList(contract),
                title="Static PDF contract",
            ),
            Panel(
                el(
                    "ol",
                    [el("li", layer) for layer in data["css_layers"]],
                    class_name="ref-layer-list",
                ),
                title="CSS layer order",
            ),
            class_name="ref-two-column ref-contract-grid",
        ),
        class_name="ref-architecture-page",
        page_id="architecture",
    )


def PaginationPage(data: dict[str, Any]) -> object:
    page_columns = [
        ("page", "Page"),
        ("page_id", "Page ID"),
        ("section", "Section"),
        ("break_rule", "Break rule"),
    ]
    manual_contract = [
        ("Page object", "Every planned page is an explicit Page(...) component."),
        ("Break rule", ".ref-page sets min-height: 11in and page-break-after: always."),
        ("Geometry", "The engine uses zero margins; each page owns its visible padding."),
        ("Appendix split", "Coverage and evidence are separate authored pages, not incidental overflow."),
    ]
    footer_contract = [
        ("footer_first", "Cover page label plus per-page weight placeholder."),
        ("footer_each", "Page number, page count, event count, and per-page weight."),
        ("footer_last", "Final page label with document total count and total weight."),
        ("Page data source", "paginated_context reads data-fb values from ledger rows."),
    ]
    return Page(
        PageHeader(
            "Pagination",
            "Manual page plan and engine footers",
            "The scaffold shows both the authored page boundaries and the PDF footer templates that the engine applies.",
        ),
        Panel(
            DataTable(
                page_columns,
                data["page_plan"],
                caption="Authored manual page sequence",
                class_name="ref-table-compact ref-page-plan-table",
            ),
            title="Manual pagination map",
            class_name="ref-panel-table ref-panel-wide ref-panel-page-plan",
        ),
        el(
            "section",
            Panel(
                KeyValueList(manual_contract),
                title="Manual pagination contract",
            ),
            Panel(
                KeyValueList(footer_contract),
                title="Footer contract",
            ),
            class_name="ref-two-column ref-pagination-grid",
        ),
        Panel(
            el(
                "pre",
                (
                    'footer_each =\n'
                    '  "Fullbleed canonical reference | manual page {page} of {pages}"\n'
                    '  "events {count:reference.event}"\n'
                    '  "page weight {sum:reference.weight}"\n'
                    'footer_last =\n'
                    '  "Fullbleed canonical reference | final page {page} of {pages}"\n'
                    '  "total events {total_count:reference.event}"\n'
                    '  "total weight {total:reference.weight}"'
                ),
                class_name="ref-code-block",
            ),
            title="Footer templates rendered by PdfEngine",
            class_name="ref-footer-template-panel",
        ),
        class_name="ref-pagination-page",
        page_id="pagination-footers",
    )
