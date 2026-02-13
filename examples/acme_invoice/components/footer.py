from .fb_ui import component, el
from .primitives import Text


@component
def Footer(*, subtotal: str, tax_label: str, tax_amount: str, total: str):
    def _row(label: str, value: str, *, grand: bool = False):
        row_class = "fb-total-row fb-total-grand" if grand else "fb-total-row"
        return el(
            "div",
            Text(label, tag="span", class_name="fb-total-label"),
            Text(value, tag="span", class_name="fb-total-value"),
            class_name=row_class,
        )

    return el(
        "footer",
        _row("Subtotal", subtotal),
        _row(tax_label, tax_amount),
        _row("Total", total, grand=True),
        class_name="fb-totals",
        data_fb_role="footer",
    )
