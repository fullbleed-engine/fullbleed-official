from __future__ import annotations

from typing import Any, Iterable

from .core import component, el
from .style import merge_style_attr_values

# Scaffold primitives intentionally restrict `tag=` overrides to the engine-safe
# HTML subset implemented in `src/html.rs`.
# This keeps component authoring aligned with engine behavior and avoids
# accidental browser-only constructs.

ENGINE_SAFE_CONTAINER_TAGS = {
    "div",
    "section",
    "article",
    "header",
    "footer",
    "aside",
    "nav",
    "main",
    "blockquote",
    "dl",
    "dt",
    "dd",
}

ENGINE_SAFE_TEXT_TAGS = {
    "span",
    "p",
    "small",
    "strong",
    "em",
    "b",
    "i",
    "u",
    "code",
    "label",
    "a",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "aside",
    "nav",
    "main",
    "blockquote",
}

ENGINE_SAFE_INLINE_TAGS = {
    "span",
    "a",
    "small",
    "strong",
    "em",
    "b",
    "i",
    "u",
    "code",
    "label",
}

ENGINE_SAFE_SECTION_HEADER_TAGS = {
    "header",
    "div",
    "section",
    "article",
}

ENGINE_SAFE_TITLE_TAGS = {
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "span",
    "div",
}

ENGINE_SAFE_LIST_TAGS = {
    "ul",
    "ol",
}

ENGINE_SAFE_BADGE_TAGS = {
    "span",
    "small",
    "strong",
    "em",
}

ENGINE_SAFE_VALUE_TAGS = {
    "span",
    "small",
    "strong",
    "em",
    "b",
    "i",
    "label",
    "p",
}


def _merge_classes(*parts: Any) -> str:
    values: list[str] = []
    for part in parts:
        if not part:
            continue
        text = str(part).strip()
        if text:
            values.append(text)
    return " ".join(values)


def _apply_class(props: dict[str, Any], base_class: str | None, class_name: str | None) -> None:
    existing = props.pop("class_name", None)
    merged = _merge_classes(existing, base_class, class_name)
    if merged:
        props["class_name"] = merged


def _merge_style(props: dict[str, Any], style_fragment: str | None) -> None:
    if not style_fragment:
        return
    props["style"] = merge_style_attr_values(props.get("style"), style_fragment)


def _normalize_tag(tag: str) -> str:
    return str(tag).strip().lower()


def _require_tag(tag: str, *, allowed: set[str], primitive: str) -> str:
    normalized = _normalize_tag(tag)
    if normalized in allowed:
        return normalized
    choices = ", ".join(sorted(allowed))
    raise ValueError(
        f"{primitive} tag {tag!r} is outside the scaffold engine-safe subset. "
        f"Allowed: {choices}"
    )


@component
def Text(content: Any, *, tag: str = "span", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_TEXT_TAGS, primitive="Text")
    _apply_class(props, None, class_name)
    return el(tag, content, **props)


@component
def Box(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="Box")
    _apply_class(props, "ui-box", class_name)
    return el(tag, list(children), **props)


@component
def Stack(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="Stack")
    _apply_class(props, "ui-stack", class_name)
    return el(tag, list(children), **props)


@component
def Row(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="Row")
    _apply_class(props, "ui-row", class_name)
    return el(tag, list(children), **props)


@component
def LayoutGrid(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="LayoutGrid")
    _apply_class(props, "ui-layout-grid", class_name)
    return el(tag, list(children), **props)


@component
def Inline(*children: Any, tag: str = "span", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_INLINE_TAGS, primitive="Inline")
    _apply_class(props, "ui-inline", class_name)
    return el(tag, list(children), **props)


@component
def Center(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="Center")
    _apply_class(props, "ui-center", class_name)
    return el(tag, list(children), **props)


@component
def Card(*children: Any, tag: str = "section", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_CONTAINER_TAGS, primitive="Card")
    _apply_class(props, "ui-card", class_name)
    return el(tag, list(children), **props)


@component
def Spacer(
    *,
    block: str = "0.5rem",
    inline: str | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    _apply_class(props, "ui-spacer", class_name)
    _merge_style(props, f"height: {block};")
    if inline:
        _merge_style(props, f"width: {inline};")
    props.setdefault("aria_hidden", "true")
    return el("div", **props)


@component
def Divider(*, tag: str = "hr", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed={"hr"}, primitive="Divider")
    _apply_class(props, "ui-divider", class_name)
    return el(tag, **props)


@component
def SectionHeader(
    title: str,
    *,
    subtitle: str | None = None,
    kicker: str | None = None,
    tag: str = "header",
    class_name: str | None = None,
    title_tag: str = "h2",
    subtitle_tag: str = "p",
    kicker_tag: str = "p",
    **props: Any,
) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_SECTION_HEADER_TAGS, primitive="SectionHeader")
    title_tag = _require_tag(title_tag, allowed=ENGINE_SAFE_TITLE_TAGS, primitive="SectionHeader.title_tag")
    subtitle_tag = _require_tag(
        subtitle_tag,
        allowed=ENGINE_SAFE_VALUE_TAGS,
        primitive="SectionHeader.subtitle_tag",
    )
    kicker_tag = _require_tag(
        kicker_tag,
        allowed=ENGINE_SAFE_VALUE_TAGS,
        primitive="SectionHeader.kicker_tag",
    )
    nodes: list[object] = []
    if kicker:
        nodes.append(Text(kicker, tag=kicker_tag, class_name="ui-kicker"))
    nodes.append(Text(title, tag=title_tag, class_name="ui-title"))
    if subtitle:
        nodes.append(Text(subtitle, tag=subtitle_tag, class_name="ui-subtitle"))
    _apply_class(props, "ui-section-header", class_name)
    return el(tag, nodes, **props)


@component
def List(*items: Any, tag: str = "ul", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_LIST_TAGS, primitive="List")
    _apply_class(props, "ui-list", class_name)
    return el(tag, list(items), **props)


@component
def ListItems(
    values: Iterable[Any],
    *,
    class_name: str | None = None,
    item_class: str | None = None,
    ordered: bool = False,
    **props: Any,
) -> object:
    tag = "ol" if ordered else "ul"
    items = [ListItem(value, class_name=item_class) for value in values]
    return List(*items, tag=tag, class_name=class_name, **props)


@component
def ListItem(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-list-item", class_name)
    return el("li", list(children), **props)


@component
def Table(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-table", class_name)
    return el("table", list(children), **props)


@component
def THead(*rows: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-thead", class_name)
    return el("thead", list(rows), **props)


@component
def TBody(*rows: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-tbody", class_name)
    return el("tbody", list(rows), **props)


@component
def Tr(*cells: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-tr", class_name)
    return el("tr", list(cells), **props)


@component
def Th(content: Any, *, class_name: str | None = None, scope: str | None = None, **props: Any) -> object:
    if scope:
        props["scope"] = scope
    _apply_class(props, "ui-th", class_name)
    return el("th", content, **props)


@component
def Td(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-td", class_name)
    return el("td", content, **props)


@component
def Badge(content: Any, *, tag: str = "span", class_name: str | None = None, **props: Any) -> object:
    tag = _require_tag(tag, allowed=ENGINE_SAFE_BADGE_TAGS, primitive="Badge")
    _apply_class(props, "ui-badge", class_name)
    return el(tag, content, **props)


@component
def KeyValueRow(
    label: Any,
    value: Any,
    *,
    class_name: str | None = None,
    label_class: str | None = None,
    value_class: str | None = None,
    label_tag: str = "span",
    value_tag: str = "span",
    **props: Any,
) -> object:
    label_tag = _require_tag(label_tag, allowed=ENGINE_SAFE_VALUE_TAGS, primitive="KeyValueRow.label_tag")
    value_tag = _require_tag(value_tag, allowed=ENGINE_SAFE_VALUE_TAGS, primitive="KeyValueRow.value_tag")
    return Row(
        Text(label, tag=label_tag, class_name=_merge_classes("ui-kv-label", label_class)),
        Text(value, tag=value_tag, class_name=_merge_classes("ui-kv-value", value_class)),
        class_name=_merge_classes("ui-key-value", class_name),
        **props,
    )


@component
def PriceRow(
    label: Any,
    value: Any,
    *,
    class_name: str | None = None,
    label_class: str | None = None,
    value_class: str | None = None,
    label_tag: str = "span",
    value_tag: str = "span",
    **props: Any,
) -> object:
    label_tag = _require_tag(label_tag, allowed=ENGINE_SAFE_VALUE_TAGS, primitive="PriceRow.label_tag")
    value_tag = _require_tag(value_tag, allowed=ENGINE_SAFE_VALUE_TAGS, primitive="PriceRow.value_tag")
    return Row(
        Text(label, tag=label_tag, class_name=_merge_classes("ui-price-label", label_class)),
        Text(value, tag=value_tag, class_name=_merge_classes("ui-price-value", value_class)),
        class_name=_merge_classes("ui-price-row", class_name),
        **props,
    )


@component
def IconLabel(
    icon: object,
    body: object,
    *,
    class_name: str | None = None,
    icon_class: str | None = None,
    body_class: str | None = None,
    **props: Any,
) -> object:
    return Row(
        Box(icon, class_name=_merge_classes("ui-icon", icon_class)),
        Box(body, class_name=_merge_classes("ui-label-body", body_class)),
        class_name=_merge_classes("ui-icon-label", class_name),
        **props,
    )
