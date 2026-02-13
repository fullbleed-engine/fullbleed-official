from __future__ import annotations

from dataclasses import dataclass

from .fb_ui import component, el
from .primitives import PriceRow, Stack, Text


@dataclass(frozen=True)
class MenuItem:
    name: str
    price: str
    description: str


@dataclass(frozen=True)
class MenuSection:
    title: str
    items: tuple[MenuItem, ...]


def _render_item(item: MenuItem) -> object:
    return el(
        "li",
        PriceRow(
            item.name,
            item.price,
            class_name="item-head",
            label_class="item-name",
            value_class="item-price",
        ),
        Text(item.description, tag="div", class_name="item-desc"),
        class_name="menu-item",
    )


@component
def CenterPanel(*, sections: list[MenuSection]) -> object:
    section_nodes = []
    for section in sections:
        section_nodes.append(
            Stack(
                Text(section.title, tag="h3", class_name="section-title"),
                el("ul", [_render_item(item) for item in section.items], class_name="menu-list"),
                tag="section",
                class_name="menu-section",
            )
        )

    return Stack(
        *section_nodes,
        tag="section",
        class_name="center-panel",
        data_fb_role="coastal-center",
    )
