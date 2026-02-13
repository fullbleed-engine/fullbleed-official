from .fb_ui import component, el
from .primitives import Text


def _bank_icon():
    return el(
        "svg",
        el("rect", x="2", y="3", width="20", height="18", rx="3"),
        el("path", d="M2 10 H22"),
        el("path", d="M7 10 V21"),
        el("path", d="M12 10 V21"),
        el("path", d="M17 10 V21"),
        el("path", d="M5 3 V1.8 H19 V3"),
        viewBox="0 0 24 24",
        class_name="bs-bank-icon",
        fill="none",
        stroke="#2f6ff5",
        stroke_width="1.8",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _phone_icon():
    return el(
        "svg",
        el("path", d="M5.3 2.8 C4.9 2.8 4.6 3 4.4 3.4 L3.8 4.8 C3.5 5.6 3.7 6.5 4.3 7.1 L8.9 11.7 C9.5 12.3 10.4 12.5 11.2 12.2 L12.6 11.6 C13 11.4 13.2 11.1 13.2 10.7 V8.8 C13.2 8.4 12.9 8.1 12.5 8 L10.7 7.6 C10.3 7.5 9.9 7.7 9.7 8 L9.3 8.7 L7.3 6.7 L8 6.3 C8.3 6.1 8.5 5.7 8.4 5.3 L8 3.5 C7.9 3.1 7.6 2.8 7.2 2.8 Z"),
        viewBox="0 0 16 16",
        class_name="bs-contact-icon",
        fill="none",
        stroke="#6a7483",
        stroke_width="1.25",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _mail_icon():
    return el(
        "svg",
        el("rect", x="2", y="3.3", width="12", height="9.4", rx="1.4"),
        el("path", d="M2.8 4.3 L8 8.2 L13.2 4.3"),
        viewBox="0 0 16 16",
        class_name="bs-contact-icon",
        fill="none",
        stroke="#6a7483",
        stroke_width="1.3",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _globe_icon():
    return el(
        "svg",
        el("circle", cx="8", cy="8", r="6"),
        el("path", d="M2.4 8 H13.6"),
        el("path", d="M8 2.1 C9.6 3.7 10.5 6.2 10.5 8 C10.5 9.8 9.6 12.3 8 13.9"),
        el("path", d="M8 2.1 C6.4 3.7 5.5 6.2 5.5 8 C5.5 9.8 6.4 12.3 8 13.9"),
        viewBox="0 0 16 16",
        class_name="bs-contact-icon",
        fill="none",
        stroke="#6a7483",
        stroke_width="1.25",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


def _contact_line(icon: object, value: str):
    return el(
        "tr",
        el("td", icon, class_name="bs-contact-icon-cell"),
        el("td", Text(value, tag="span", class_name="bs-contact-value"), class_name="bs-contact-text-cell"),
    )


def _account_detail(label: str, value: str, *, emphasize: bool = False):
    value_class = "bs-detail-value bs-detail-emphasis" if emphasize else "bs-detail-value"
    return el(
        "p",
        Text(f"{label}:", tag="span", class_name="bs-detail-label"),
        Text(value, tag="span", class_name=value_class),
        class_name="bs-detail-row",
    )


@component
def Header(*, meta: dict[str, str]):
    return el(
        "section",
        el(
            "header",
            el(
                "table",
                el(
                    "tbody",
                    el(
                        "tr",
                        el(
                            "td",
                            el(
                                "table",
                                el(
                                    "tbody",
                                    el(
                                        "tr",
                                        el("td", _bank_icon(), class_name="bs-brand-icon-cell"),
                                        el(
                                            "td",
                                            el(
                                                "div",
                                                Text(meta["bank_name"], tag="h1", class_name="bs-bank-name"),
                                                Text(meta["bank_tagline"], tag="p", class_name="bs-bank-tagline"),
                                                class_name="bs-brand-copy",
                                            ),
                                            class_name="bs-brand-text-cell",
                                        ),
                                    ),
                                ),
                                class_name="bs-brand-table",
                            ),
                            class_name="bs-top-left",
                        ),
                        el(
                            "td",
                            el(
                                "table",
                                el(
                                    "tbody",
                                    _contact_line(_phone_icon(), meta["contact_phone"]),
                                    _contact_line(_mail_icon(), meta["contact_email"]),
                                    _contact_line(_globe_icon(), meta["contact_website"]),
                                ),
                                class_name="bs-contact-table",
                            ),
                            class_name="bs-top-right",
                        ),
                    ),
                ),
                class_name="bs-top-table",
            ),
        ),
        el("div", class_name="bs-accent-rule"),
        el(
            "section",
            el(
                "div",
                Text("ACCOUNT HOLDER", tag="p", class_name="bs-kicker"),
                Text(meta["account_holder"], tag="p", class_name="bs-holder-name"),
                Text(meta["account_address_line1"], tag="p", class_name="bs-address-line"),
                Text(meta["account_address_line2"], tag="p", class_name="bs-address-line"),
                class_name="bs-account-col bs-account-holder",
            ),
            el(
                "div",
                Text("ACCOUNT DETAILS", tag="p", class_name="bs-kicker"),
                _account_detail("Account Number", meta["account_number"]),
                _account_detail("Routing Number", meta["routing_number"]),
                _account_detail("Statement Period", meta["statement_period"], emphasize=True),
                class_name="bs-account-col bs-account-details",
            ),
            class_name="bs-account-grid",
        ),
        class_name="bs-header",
        data_fb_role="header",
    )
