from __future__ import annotations

from typing import Any

from .fb_ui import component, el


@component
def Text(content: str, *, tag: str = "span", class_name: str | None = None, **props: Any) -> object:
    if class_name is not None:
        props["class_name"] = class_name
    return el(tag, content, **props)


@component
def Stack(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    if class_name is not None:
        props["class_name"] = class_name
    return el(tag, list(children), **props)


@component
def Row(*children: Any, tag: str = "div", class_name: str | None = None, **props: Any) -> object:
    classes = "ui-row"
    if class_name:
        classes = f"{classes} {class_name}"
    props["class_name"] = classes
    return el(tag, list(children), **props)


@component
def PriceRow(
    label: str,
    value: str,
    *,
    class_name: str,
    label_class: str,
    value_class: str,
    label_tag: str = "span",
    value_tag: str = "span",
) -> object:
    return Row(
        Text(label, tag=label_tag, class_name=label_class),
        Text(value, tag=value_tag, class_name=value_class),
        class_name=class_name,
    )


@component
def IconLabel(icon: object, body: object, *, class_name: str, icon_class: str = "ui-icon", body_class: str = "ui-body") -> object:
    return Row(
        Stack(icon, class_name=icon_class),
        Stack(body, class_name=body_class),
        class_name=class_name,
    )
