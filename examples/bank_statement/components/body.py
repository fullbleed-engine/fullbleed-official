from .fb_ui import component, el
from .primitives import TBody, Table, Td, Text, Th, THead, Tr


def _summary_item(item: dict[str, str]):
    tone = item.get("tone", "neutral")
    return el(
        "div",
        Text(item.get("label", ""), tag="p", class_name="bs-summary-label"),
        Text(
            item.get("value", ""),
            tag="p",
            class_name=f"bs-summary-value bs-summary-{tone}",
        ),
        class_name="bs-summary-item",
    )


@component
def Body(*, summary: list[dict[str, str]], transactions: list[dict[str, str]]):
    tx_rows: list[object] = []
    for row in transactions:
        amount_class = row.get("amount_class", "neutral")
        tx_rows.append(
            Tr(
                Td(row.get("date", ""), class_name="bs-col-date"),
                Td(row.get("description", ""), class_name="bs-col-description"),
                Td(row.get("amount", ""), class_name=f"bs-col-amount bs-tx-{amount_class}"),
                Td(row.get("balance", ""), class_name="bs-col-balance"),
                class_name="bs-transaction-row",
                data_fb=f"tx.amount={row.get('amount_raw', '0')}",
            )
        )

    return el(
        "section",
        el(
            "section",
            Text("ACCOUNT SUMMARY", tag="p", class_name="bs-section-kicker"),
            el("div", [_summary_item(item) for item in summary], class_name="bs-summary-grid"),
            class_name="bs-summary-block",
        ),
        el(
            "section",
            Text("TRANSACTION HISTORY", tag="p", class_name="bs-section-kicker"),
            Table(
                THead(
                    Tr(
                        Th("DATE", class_name="bs-head-date"),
                        Th("DESCRIPTION", class_name="bs-head-description"),
                        Th("AMOUNT", class_name="bs-head-amount"),
                        Th("BALANCE", class_name="bs-head-balance"),
                    )
                ),
                TBody(*tx_rows),
                class_name="bs-transaction-table",
            ),
            class_name="bs-history-block",
        ),
        class_name="bs-body",
        data_fb_role="body",
    )
