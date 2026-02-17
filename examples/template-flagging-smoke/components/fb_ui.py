from __future__ import annotations

from dataclasses import dataclass, field
from html import escape
from typing import Any


@dataclass
class Element:
    tag: str
    props: dict[str, Any] = field(default_factory=dict)
    children: list[Any] = field(default_factory=list)


def el(tag: str, *children: Any, **props: Any) -> Element:
    flat: list[Any] = []
    for child in children:
        if child is None:
            continue
        if isinstance(child, (list, tuple)):
            flat.extend(x for x in child if x is not None)
        else:
            flat.append(child)
    return Element(tag=tag, props=props, children=flat)


def _normalize_attr_name(name: str) -> str:
    if name == "class_name":
        return "class"
    if name.startswith("data_fb_"):
        return "data-fb-" + name[len("data_fb_") :].replace("_", "-")
    return name.replace("_", "-")


def _render_attrs(props: dict[str, Any]) -> str:
    parts: list[str] = []
    for key, value in props.items():
        if value is None or value is False:
            continue
        attr = _normalize_attr_name(key)
        if value is True:
            parts.append(attr)
        else:
            parts.append(f'{attr}="{escape(str(value), quote=True)}"')
    return (" " + " ".join(parts)) if parts else ""


def render_node(node: Any) -> str:
    if node is None:
        return ""
    if isinstance(node, Element):
        attrs = _render_attrs(node.props)
        children_html = "".join(render_node(child) for child in node.children)
        return f"<{node.tag}{attrs}>{children_html}</{node.tag}>"
    return escape(str(node))

