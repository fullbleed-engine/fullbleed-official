from __future__ import annotations

from dataclasses import dataclass

from .fb_ui import component, el
from .primitives import Row, Stack, Text
from styles.tokens import TOKENS


@dataclass(frozen=True)
class BeverageItem:
    name: str
    price: str
    description: str


def _s(*parts: str) -> str:
    return "; ".join(parts)


def _render_beverage(item: BeverageItem) -> object:
    return el(
        "li",
        el(
            "div",
            Text(
                item.name,
                tag="span",
                class_name="beverage-name",
                style=_s("display: table-cell", "color: #f7fbff", "font-weight: 600"),
            ),
            Text(
                item.price,
                tag="span",
                class_name="beverage-price",
                style=_s(
                    "display: table-cell",
                    "width: 0.65in",
                    "text-align: right",
                    "color: #1fa5f1",
                    "font-weight: 520",
                ),
            ),
            class_name="beverage-head",
            style=_s("display: table", "width: 100%", "font-size: 0.188in", "line-height: 1.18"),
        ),
        Text(
            item.description,
            tag="div",
            class_name="beverage-desc",
            style=_s(
                "margin-top: 0.03in",
                "color: #9fb8d2",
                "font-size: 0.14in",
                "line-height: 1.26",
            ),
        ),
        class_name="beverage-item",
        style=_s("margin: 0 0 0.20in"),
    )


def _beverage_icon():
    return el(
        "svg",
        el("path", d="M3.7 2 H12.3 L11.2 6.7 C10.8 8.6 9.6 9.8 8 9.8 C6.4 9.8 5.2 8.6 4.8 6.7 Z"),
        el("path", d="M8 9.8 V13.4"),
        el("path", d="M6 13.5 H10"),
        viewBox="0 0 16 16",
        class_name="beverage-icon",
        fill="none",
        stroke=TOKENS.accent,
        stroke_width="1.7",
        stroke_linecap="round",
        stroke_linejoin="round",
        aria_hidden="true",
        style=_s(
            "display: inline-block",
            "width: 0.24in",
            "height: 0.24in",
            "margin-right: 0.10in",
            "vertical-align: middle",
        ),
    )


def _wave_mark(*, class_name: str, style: str | None = None):
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
        style=style,
    )


def _rule_line(*, class_name: str, stroke: str, width: str, height: str):
    return el(
        "svg",
        el(
            "line",
            x1="0",
            y1="1",
            x2="100",
            y2="1",
            stroke=stroke,
            stroke_width="2",
            stroke_linecap="square",
        ),
        viewBox="0 0 100 2",
        width=width,
        height=height,
        class_name=class_name,
        fill="none",
        preserveAspectRatio="none",
        aria_hidden="true",
        style=_s("display: block"),
    )


@component
def RightPanel(*, beverages: list[BeverageItem]) -> object:
    return Stack(
        el(
            "div",
            _wave_mark(
                class_name="wave-mark-right",
                style=_s(
                    "display: block",
                    "width: 0.62in",
                    "height: 0.62in",
                ),
            ),
            class_name="wave-mark-wrap-right",
            style=_s(
                "display: flex",
                "justify-content: center",
                "align-items: flex-start",
                "width: 100%",
                "height: 0.62in",
                "margin: 0 0 0.48in",
            ),
        ),
        el(
            "div",
            el(
                "table",
                el(
                    "tr",
                    el(
                        "td",
                        "The Coastal",
                        style=_s(
                            "padding: 0",
                            "font-family: 'Times New Roman', Georgia, serif",
                            "font-size: 0.72in",
                            "font-weight: 500",
                            "line-height: 0.90",
                            "text-align: center",
                            "color: #e7f4ff",
                        ),
                    ),
                ),
                el(
                    "tr",
                    el(
                        "td",
                        "Table",
                        style=_s(
                            "padding: 0",
                            "font-family: 'Times New Roman', Georgia, serif",
                            "font-size: 0.72in",
                            "font-weight: 500",
                            "line-height: 0.90",
                            "text-align: center",
                            "color: #e7f4ff",
                        ),
                    ),
                ),
                class_name="hero-title",
                style=_s(
                    "margin: 0",
                    "display: inline-table",
                    "border-collapse: collapse",
                ),
            ),
            style=_s("width: 100%", "text-align: center"),
        ),
        Text(
            "Where Ocean Meets Elegance",
            tag="p",
            class_name="hero-tagline",
            style=_s(
                "margin: 0.16in 0 0.15in",
                "font-size: 0.15in",
                "line-height: 1.2",
                "font-weight: 420",
                "text-align: center",
                "color: #c8d9ee",
            ),
        ),
        el(
            "div",
            _rule_line(class_name="hero-accent", stroke="#1fa5f1", width="0.95in", height="0.03in"),
            class_name="hero-accent-wrap",
            style=_s(
                "display: flex",
                "justify-content: center",
                "width: 100%",
                "margin: 0 auto 0.26in",
            ),
        ),
        el(
            "div",
            _rule_line(class_name="hero-divider", stroke="#4f6687", width="4.48in", height="0.012in"),
            class_name="hero-divider-wrap",
            style=_s(
                "display: flex",
                "justify-content: center",
                "width: 100%",
                "margin: 0 auto 0.32in",
            ),
        ),
        Stack(
            Row(
                _beverage_icon(),
                Text(
                    "Beverages",
                    tag="h2",
                    class_name="beverage-title",
                    style=_s(
                        "margin: 0",
                        "display: inline",
                        "font-family: 'Times New Roman', Georgia, serif",
                        "font-size: 0.52in",
                        "line-height: 1.06",
                        "font-weight: 600",
                        "vertical-align: middle",
                        "color: #f7fbff",
                    ),
                ),
                class_name="beverage-header",
                style=_s("width: 100%", "margin-bottom: 0.05in"),
            ),
            Text(
                "SIGNATURE COCKTAILS",
                tag="p",
                class_name="beverage-kicker",
                style=_s(
                    "margin: 0 0 0.24in",
                    "font-size: 0.145in",
                    "font-weight: 520",
                    "color: #1fa5f1",
                    "line-height: 1.2",
                    "text-transform: uppercase",
                ),
            ),
            el(
                "ul",
                [_render_beverage(item) for item in beverages],
                class_name="beverage-list",
                style=_s("list-style: none", "margin: 0", "padding: 0"),
            ),
            tag="section",
            class_name="beverage-block",
        ),
        tag="aside",
        class_name="right-panel",
        data_fb_role="coastal-right",
        style=_s(
            "display: table-cell",
            "width: 5.8in",
            "height: 8.5in",
            "vertical-align: top",
            "box-sizing: border-box",
            "background: #121f3f",
            "color: #e7f4ff",
            "padding: 0.68in 0.66in 0.48in",
        ),
    )
