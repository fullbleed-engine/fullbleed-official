from .fb_ui import component
from .primitives import Card, SectionHeader, TBody, Table, Td, Text, Th, THead, Tr


@component
def Body(*, rows: list[dict[str, str]]):
    table_rows: list[object] = []
    for row in rows:
        table_rows.append(
            Tr(
                Th(row.get("label", ""), scope="row", class_name="fb-summary-label"),
                Td(row.get("value", ""), class_name="fb-summary-value"),
                Td(row.get("note", ""), class_name="fb-summary-note"),
                class_name="fb-summary-row",
                data_fb_role="summary-row",
            )
        )

    return Card(
        SectionHeader("Summary", title_tag="h2", class_name="fb-body-title"),
        Text(
            "Structured data rendered with component primitives.",
            tag="p",
            class_name="fb-body-intro",
        ),
        Table(
            THead(
                Tr(
                    Th("Field"),
                    Th("Value"),
                    Th("Note"),
                ),
            ),
            TBody(*table_rows),
            class_name="fb-summary-table",
        ),
        tag="section",
        class_name="fb-body-card",
        data_fb_role="body",
    )
