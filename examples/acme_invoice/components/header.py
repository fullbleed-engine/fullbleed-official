from .fb_ui import component, el
from .primitives import Text


def _icon_mail():
    return el(
        "svg",
        el("rect", x="1.6", y="3.2", width="12.8", height="9.6", rx="1.6"),
        el("path", d="M2.4 4.2 L8 8.8 L13.6 4.2"),
        viewBox="0 0 16 16",
        class_name="fb-icon",
        fill="none",
        stroke="#6f7781",
        stroke_width="1.6",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _icon_globe():
    return el(
        "svg",
        el("circle", cx="8", cy="8", r="6.4"),
        el("path", d="M1.8 8h12.4"),
        el("path", d="M8 1.8c1.6 1.4 2.6 3.8 2.6 6.2s-1 4.8-2.6 6.2"),
        el("path", d="M8 1.8C6.4 3.2 5.4 5.6 5.4 8s1 4.8 2.6 6.2"),
        viewBox="0 0 16 16",
        class_name="fb-icon",
        fill="none",
        stroke="#6f7781",
        stroke_width="1.6",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _icon_calendar():
    return el(
        "svg",
        el("rect", x="1.8", y="3.2", width="12.4", height="11", rx="1.4"),
        el("line", x1="4.8", y1="1.8", x2="4.8", y2="4.6"),
        el("line", x1="11.2", y1="1.8", x2="11.2", y2="4.6"),
        el("line", x1="1.8", y1="6.2", x2="14.2", y2="6.2"),
        viewBox="0 0 16 16",
        class_name="fb-icon",
        fill="none",
        stroke="#6f7781",
        stroke_width="1.6",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _icon_line(icon: object, value: str):
    return el(
        "p",
        icon,
        Text(value, tag="span"),
        class_name="fb-icon-line",
    )


@component
def Header(*, invoice: dict[str, str]):
    return el(
        "section",
        el(
            "header",
            el(
                "section",
                Text(invoice["studio_name"], tag="h1", class_name="fb-brand"),
                Text(invoice["studio_tagline"], tag="p", class_name="fb-tagline"),
                class_name="fb-brand-block",
            ),
            el(
                "section",
                Text("INVOICE", tag="span", class_name="fb-badge"),
                el(
                    "p",
                    Text("#", tag="span", class_name="fb-prefix"),
                    Text(invoice["invoice_number"], tag="span", class_name="fb-value"),
                    class_name="fb-meta-line fb-number-line",
                ),
                el(
                    "p",
                    _icon_calendar(),
                    Text(invoice["invoice_date"], tag="span"),
                    class_name="fb-meta-line fb-date-line",
                ),
                class_name="fb-meta-block",
            ),
            class_name="fb-top-bar",
        ),
        el(
            "section",
            el(
                "section",
                Text("FROM", tag="p", class_name="fb-party-kicker"),
                Text(invoice["from_company"], tag="p", class_name="fb-party-company"),
                _icon_line(_icon_mail(), invoice["from_email"]),
                _icon_line(_icon_globe(), invoice["from_website"]),
                Text(invoice["from_address_line1"], tag="p", class_name="fb-plain-line"),
                Text(invoice["from_address_line2"], tag="p", class_name="fb-plain-line"),
                class_name="fb-party fb-party-from",
            ),
            el(
                "section",
                Text("BILL TO", tag="p", class_name="fb-party-kicker"),
                Text(invoice["bill_company"], tag="p", class_name="fb-party-company"),
                Text(invoice["bill_contact"], tag="p", class_name="fb-plain-line"),
                Text(invoice["bill_email"], tag="p", class_name="fb-plain-line"),
                Text(invoice["bill_address_line1"], tag="p", class_name="fb-plain-line"),
                Text(invoice["bill_address_line2"], tag="p", class_name="fb-plain-line"),
                class_name="fb-party fb-party-billto",
            ),
            class_name="fb-party-grid",
        ),
        class_name="fb-header",
        data_fb_role="header",
    )
