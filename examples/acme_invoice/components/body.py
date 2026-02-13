from .fb_ui import component, el
from .primitives import TBody, Table, Td, Th, THead, Tr


@component
def Body(*, items: list[dict[str, str]]):
    table_rows: list[object] = []
    for row in items:
        table_rows.append(
            Tr(
                Td(row.get("description", ""), class_name="fb-col-desc"),
                Td(row.get("qty", ""), class_name="fb-col-qty"),
                Td(row.get("rate", ""), class_name="fb-col-rate"),
                Td(row.get("amount", ""), class_name="fb-col-amount"),
                class_name="fb-item-row",
            )
        )

    return el(
        "section",
        Table(
            THead(
                Tr(
                    Th("DESCRIPTION", class_name="fb-col-desc"),
                    Th("QTY", class_name="fb-col-qty"),
                    Th("RATE", class_name="fb-col-rate"),
                    Th("AMOUNT", class_name="fb-col-amount"),
                ),
            ),
            TBody(*table_rows),
            class_name="fb-items-table",
        ),
        class_name="fb-items-wrap",
        data_fb_role="body",
    )
