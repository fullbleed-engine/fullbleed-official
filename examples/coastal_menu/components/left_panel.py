from __future__ import annotations

from dataclasses import dataclass

from .fb_ui import component, el
from .primitives import Stack, Text
from styles.tokens import TOKENS


@dataclass(frozen=True)
class ContactRow:
    icon: str
    lines: tuple[str, ...]


def _icon(kind: str):
    if kind == "location":
        return el(
            "svg",
            el("circle", cx="8", cy="6", r="3.3"),
            el("path", d="M8 9.3 L8 14"),
            viewBox="0 0 16 16",
            class_name="contact-icon-svg",
            fill="none",
            stroke=TOKENS.white_soft,
            stroke_width="1.7",
            stroke_linecap="round",
            stroke_linejoin="round",
            aria_hidden="true",
        )
    if kind == "clock":
        return el(
            "svg",
            el("circle", cx="8", cy="8", r="6.2"),
            el("path", d="M8 4.3 V8 L10.6 9.5"),
            viewBox="0 0 16 16",
            class_name="contact-icon-svg",
            fill="none",
            stroke=TOKENS.white_soft,
            stroke_width="1.55",
            stroke_linecap="round",
            stroke_linejoin="round",
            aria_hidden="true",
        )
    if kind == "phone":
        return el(
            "svg",
            el("path", d="M4.2 2.8 H6.1 L7.0 5.4 L5.8 6.6 C6.5 8.0 8.0 9.5 9.4 10.2 L10.6 9.0 L13.2 9.9 V11.8 C13.2 12.4 12.7 12.9 12.1 12.9 C7.7 12.7 3.3 8.3 3.1 3.9 C3.1 3.3 3.6 2.8 4.2 2.8"),
            viewBox="0 0 16 16",
            class_name="contact-icon-svg",
            fill="none",
            stroke=TOKENS.white_soft,
            stroke_width="1.6",
            stroke_linecap="round",
            stroke_linejoin="round",
            aria_hidden="true",
        )
    if kind == "mail":
        return el(
            "svg",
            el("rect", x="1.8", y="3.1", width="12.4", height="9.8", rx="1.1"),
            el("path", d="M2.5 4.3 L8 8.3 L13.5 4.3"),
            viewBox="0 0 16 16",
            class_name="contact-icon-svg",
            fill="none",
            stroke=TOKENS.white_soft,
            stroke_width="1.35",
            stroke_linecap="round",
            stroke_linejoin="round",
            aria_hidden="true",
        )
    return el("span", "*", class_name="contact-icon-fallback")


def _wave_mark(*, class_name: str):
    return el(
        "svg",
        el("path", d="M2 3.5 C4 1.7 6 1.7 8 3.5 C10 5.3 12 5.3 14 3.5"),
        el("path", d="M2 8 C4 6.2 6 6.2 8 8 C10 9.8 12 9.8 14 8"),
        el("path", d="M2 12.5 C4 10.7 6 10.7 8 12.5 C10 14.3 12 14.3 14 12.5"),
        viewBox="0 0 16 16",
        class_name=class_name,
        fill="none",
        stroke=TOKENS.white_soft,
        stroke_width="1.35",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
    )


@component
def LeftPanel(*, rows: list[ContactRow]) -> object:
    row_nodes = []
    for row in rows:
        row_nodes.append(
            el(
                "div",
                el("div", _icon(row.icon), class_name="contact-icon-wrap"),
                el(
                    "div",
                    [Text(line, tag="p", class_name="contact-line") for line in row.lines],
                    class_name="contact-lines-wrap",
                ),
                class_name="contact-row",
            )
        )

    return Stack(
        _wave_mark(class_name="wave-mark-left"),
        Text("Visit Us", tag="h2", class_name="visit-title"),
        Stack(*row_nodes, class_name="contact-stack"),
        tag="aside",
        class_name="left-panel",
        data_fb_role="coastal-left",
    )
