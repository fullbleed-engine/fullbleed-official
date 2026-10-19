# SPDX-License-Identifier: MIT
"""Static Bootstrap components. State is authored; no scripts or event handlers."""
from __future__ import annotations

from dataclasses import dataclass, replace
from decimal import Decimal
import math

from .._style import AuthoringError, Style, pct
from .._tree import Node, _children, component
from ._core import boolean, choice, classes, identifier, link_box, list_node, node
from ._schema import BREAKPOINTS, COLORS, SIZES


@component
def Alert(*children, variant="primary", heading=None, **props):
    choice(variant, COLORS, "alert variant")
    title = node("h4", (heading,), "alert-heading") if heading is not None else None
    return node("div", (title, *children), "alert", "alert-" + variant, **props)


@component
def Badge(*children, variant="primary", pill=False, **props):
    choice(variant, COLORS, "badge variant")
    boolean(pill, "pill")
    return node("span", children, "badge", "bg-" + variant,
                "text-dark" if variant in {"warning", "info", "light"} else "",
                "rounded-pill" if pill else "", **props)


@component
def Button(*children, variant="primary", outline=False, size=None, href=None,
           disabled=False, active=False, type="button", **props):
    choice(variant, (*COLORS, "link"), "button variant")
    choice(size, SIZES, "button size", optional=True)
    choice(type, ("button", "submit", "reset"), "button type")
    for name, value in (("outline", outline), ("disabled", disabled), ("active", active)):
        boolean(value, name)
    if outline and variant == "link":
        raise AuthoringError("Link buttons do not have an outline variant", code="bootstrap.option")
    attrs = {"href": None if disabled else href, "aria-disabled": "true" if disabled else None,
             "tabindex": -1 if disabled else None} if href is not None else {"type": type, "disabled": disabled}
    result = node("a" if href is not None else "button", children, "btn",
                "btn-" + ("outline-" if outline else "") + variant,
                "btn-" + size if size else "", {"active": active, "disabled": disabled},
                attributes=attrs, **props)
    return link_box(result, result.children, None if disabled else href, inline=True)


@component
def ButtonGroup(*children, label, size=None, vertical=False, **props):
    choice(size, SIZES, "button group size", optional=True)
    boolean(vertical, "vertical")
    return node("div", children, "btn-group-vertical" if vertical else "btn-group",
                "btn-group-" + size if size else "", attributes={"role": "group", "aria-label": label}, **props)


@component
def CloseButton(*, label="Close", white=False, disabled=True, **props):
    """A static close control; disabled by default because there is no action."""
    boolean(white, "white")
    boolean(disabled, "disabled")
    return node("button", (), "btn-close", "btn-close-white" if white else "",
                attributes={"type": "button", "aria-label": label, "disabled": disabled}, **props)


@component
def Breadcrumb(*children, label="Breadcrumb", **props):
    return node("nav", (list_node("ol", children, "breadcrumb"),), attributes={"aria-label": label}, **props)


@component
def BreadcrumbItem(*children, href=None, active=False, **props):
    boolean(active, "active")
    content = (node("a", children, attributes={"href": href}),) if href is not None and not active else children
    return list_node("li", content, "breadcrumb-item", "active" if active else "",
                attributes={"aria-current": "page" if active else None}, **props)


@component(optional_parts=("header", "body", "title", "subtitle", "footer"))
def Card(*children, title=None, subtitle=None, header=None, footer=None, image=None,
         variant=None, outline=False, body=True, **props):
    """One call owns the optional header, image, body, title, subtitle and footer.

    Use body=False when composing CardBody/CardHeader/etc. explicitly.
    """
    choice(variant, COLORS, "card variant", optional=True)
    boolean(outline, "outline")
    boolean(body, "body")
    if not body and (title is not None or subtitle is not None):
        raise AuthoringError("Card title/subtitle require body=True", code="bootstrap.card")
    contents = (
        node("h2", (title,), "card-title", _part="title") if title is not None else None,
        node("h3", (subtitle,), "card-subtitle", "mb-2", "text-muted", _part="subtitle") if subtitle is not None else None,
        *children,
    )
    return node("div", (
        node("div", (header,), "card-header", _part="header") if header is not None else None,
        image,
        node("div", contents, "card-body", _part="body") if body else children,
        node("div", (footer,), "card-footer", _part="footer") if footer is not None else None,
    ), "card", ("border-" if outline else "bg-") + variant if variant else "",
       "text-white" if variant in {"primary", "secondary", "success", "danger", "dark"} and not outline else "", **props)


@component
def CardImage(*, src, alt, position="top", **props):
    from ._layout import Image
    choice(position, ("top", "bottom", "overlay"), "card image position")
    props["class_name"] = classes("card-img" + ("" if position == "overlay" else "-" + position),
                                 props.pop("class_name", None))
    return Image(src=src, alt=alt, **props)


@component
def Collapse(*children, open=True, **props):
    boolean(open, "open")
    return node("div", children, "collapse", "show" if open else "",
                attributes={"hidden": not open}, **props)


@dataclass(frozen=True, init=False)
class AccordionItem:
    title: object
    children: tuple
    key: str | None
    open: bool

    def __init__(self, title, *children, key=None, open=True):
        if key is not None:
            identifier(key)
        boolean(open, "open")
        object.__setattr__(self, "title", _children((title,)))
        object.__setattr__(self, "children", _children(children))
        object.__setattr__(self, "key", key)
        object.__setattr__(self, "open", open)


def _items(items, item_type):
    # Accept an ordered generator/list as conveniently as positional items.
    from collections.abc import Iterable, Mapping, Set
    result, active = [], set()
    def visit(value):
        if isinstance(value, item_type):
            result.append(value)
        elif isinstance(value, Iterable) and not isinstance(value, (str, bytes, Mapping, Set, Node)):
            identity = id(value)
            if identity in active:
                raise AuthoringError("Cyclic component items", code="bootstrap.items")
            active.add(identity)
            try:
                for child in value:
                    visit(child)
            finally:
                active.remove(identity)
        else:
            raise AuthoringError(f"Expected ordered {item_type.__name__} values", code="bootstrap.items")
    visit(items)
    if not result:
        raise AuthoringError("At least one item is required", code="bootstrap.items")
    keys = [item.key or f"item-{i}" for i, item in enumerate(result, 1)]
    if len(set(keys)) != len(keys):
        raise AuthoringError("Item keys must be unique, including generated keys", code="bootstrap.items")
    return tuple(zip(keys, result))


@component
def Accordion(*items, id, flush=False, heading_level=2, **props):
    identifier(id)
    boolean(flush, "flush")
    from ._layout import Heading
    content = []
    for key, item in _items(items, AccordionItem):
        heading_id, panel_id = f"{id}-{key}-heading", f"{id}-{key}-panel"
        heading = Heading(node("span", item.title, "accordion-button", "collapsed" if not item.open else ""),
                          level=heading_level, class_name="accordion-header", id=heading_id)
        panel = node("div", (node("div", item.children, "accordion-body"),), "accordion-collapse", "collapse",
                     "show" if item.open else "", attributes={"id": panel_id, "role": "region",
                     "aria-labelledby": heading_id, "hidden": not item.open})
        content.append(node("div", (heading, panel), "accordion-item"))
    return node("div", content, "accordion", "accordion-flush" if flush else "", id=id, **props)


@component
def Dropdown(label, *items, open=True, direction="down", align="start", dark=False, **props):
    boolean(open, "open")
    boolean(dark, "dark")
    choice(direction, ("down", "up", "start", "end"), "dropdown direction")
    choice(align, ("start", "end"), "dropdown alignment")
    return node("div", (
        node("div", (label,), "dropdown-toggle"),
        node("ul", items, "dropdown-menu", "position-static", "dropdown-menu-" + align,
             "dropdown-menu-dark" if dark else "", "show" if open else "", attributes={"hidden": not open}),
    ), "dropdown" if direction == "down" else "drop" + direction, **props)


@component
def DropdownItem(*children, href=None, active=False, disabled=False, **props):
    boolean(active, "active")
    boolean(disabled, "disabled")
    child = node("a" if href is not None else "span", children, "dropdown-item",
                 {"active": active, "disabled": disabled}, attributes={"href": href if not disabled else None,
                 "aria-current": "true" if active else None, "aria-disabled": "true" if disabled else None}, **props)
    return node("li", (child,))


@component
def DropdownDivider(**props):
    return node("li", (node("hr", (), "dropdown-divider", **props),))


@component
def ListGroup(*children, flush=False, horizontal=False, numbered=False, **props):
    boolean(flush, "flush")
    boolean(numbered, "numbered")
    children = _children(children)
    if numbered:
        ordinal = 0
        def number(child):
            nonlocal ordinal
            if not isinstance(child, Node):
                return child
            if child.tag is None:
                return replace(child, children=tuple(number(nested) for nested in child.children))
            if child.tag == "li":
                ordinal += 1
                printed = child._print or node("div", child.children, attributes={"role": "listitem"})
                return replace(child, _print=replace(printed, children=(f"{ordinal}. ", *printed.children)))
            return child
        children = tuple(number(child) for child in children)
    suffix = "" if horizontal is True or horizontal is False else "-" + choice(horizontal, BREAKPOINTS, "horizontal")
    return list_node("ol" if numbered else "ul", children, "list-group", "list-group-flush" if flush else "",
                "list-group-horizontal" + suffix if horizontal is not False else "",
                "list-group-numbered" if numbered else "", **props)


@component
def ListGroupItem(*children, variant=None, active=False, disabled=False, href=None, **props):
    choice(variant, COLORS, "list item variant", optional=True)
    boolean(active, "active")
    boolean(disabled, "disabled")
    contents = (node("a", children, attributes={"href": href if not disabled else None,
                           "aria-disabled": "true" if disabled else None}),) if href is not None else children
    return list_node("li", contents, "list-group-item", "list-group-item-" + variant if variant else "",
                {"active": active, "disabled": disabled, "list-group-item-action": href is not None},
                attributes={"aria-current": "true" if active else None}, **props)


@component
def Modal(*children, title=None, footer=None, id, open=True, size=None, centered=False,
          scrollable=False, **props):
    """An in-flow modal specimen, without backdrop, focus trap or close action."""
    identifier(id)
    choice(size, ("sm", "lg", "xl"), "modal size", optional=True)
    for name, value in (("open", open), ("centered", centered), ("scrollable", scrollable)):
        boolean(value, name)
    title_id = id + "-title" if title is not None else None
    content = node("div", (
        node("div", (node("h2", (title,), "modal-title", id=title_id),), "modal-header") if title is not None else None,
        node("div", children, "modal-body"),
        node("div", (footer,), "modal-footer") if footer is not None else None,
    ), "modal-content")
    dialog = node("div", (content,), "modal-dialog", "modal-" + size if size else "",
                  "modal-dialog-centered" if centered else "", "modal-dialog-scrollable" if scrollable else "")
    return node("section", (dialog,), "modal", "fb-bs-modal", "position-static", "d-block" if open else "",
                attributes={"id": id, "aria-labelledby": title_id, "hidden": not open}, **props)


@component
def Nav(*children, variant=None, vertical=False, fill=False, justified=False, **props):
    choice(variant, ("tabs", "pills"), "nav variant", optional=True)
    for name, value in (("vertical", vertical), ("fill", fill), ("justified", justified)):
        boolean(value, name)
    return list_node("ul", children, "nav", "nav-" + variant if variant else "",
                {"flex-column": vertical, "nav-fill": fill, "nav-justified": justified}, **props)


@component
def NavLink(*children, href=None, active=False, disabled=False, **props):
    boolean(active, "active")
    boolean(disabled, "disabled")
    result = node("a" if href is not None else "span", children, "nav-link", {"active": active, "disabled": disabled},
                attributes={"href": href if not disabled else None, "aria-current": "page" if active else None,
                            "aria-disabled": "true" if disabled else None}, **props)
    return link_box(result, result.children, None if disabled else href)


@dataclass(frozen=True, init=False)
class Tab:
    label: tuple
    children: tuple
    key: str | None

    def __init__(self, label, *children, key=None):
        if key is not None:
            identifier(key)
        object.__setattr__(self, "label", _children((label,)))
        object.__setattr__(self, "children", _children(children))
        object.__setattr__(self, "key", key)


def _active(items, active):
    if isinstance(active, bool) or not isinstance(active, (int, str)):
        raise AuthoringError("active requires an item key or zero-based index", code="bootstrap.active")
    keys = [key for key, _ in items]
    if isinstance(active, int):
        if not 0 <= active < len(keys):
            raise AuthoringError("active index is outside the items", code="bootstrap.active")
        return keys[active]
    if active not in keys:
        raise AuthoringError(f"Unknown active item {active!r}", code="bootstrap.active")
    return active


@component
def Tabs(*items, id, active=0, variant="tabs", **props):
    identifier(id)
    choice(variant, ("tabs", "pills"), "tabs variant")
    items = _items(items, Tab)
    active = _active(items, active)
    links, panels = [], []
    for key, item in items:
        selected = key == active
        label_id, panel_id = f"{id}-{key}-label", f"{id}-{key}-panel"
        links.append(list_node("li", (NavLink(item.label, active=selected, id=label_id),), "nav-item"))
        panels.append(TabPane(item.children, active=selected, id=panel_id,
                              attrs={"role": "region", "aria-labelledby": label_id}))
    return node("div", (Nav(links, variant=variant), node("div", panels, "tab-content")), id=id, **props)


@component
def TabPane(*children, active=True, **props):
    boolean(active, "active")
    return node("div", children, "tab-pane", "active" if active else "", attributes={"hidden": not active}, **props)


@component
def Navbar(*children, variant="light", background="light", expand=False, **props):
    choice(variant, ("light", "dark"), "navbar variant")
    choice(background, COLORS, "navbar background", optional=True)
    expansion = "navbar-expand" if expand is True else "" if expand is False else "navbar-expand-" + choice(expand, BREAKPOINTS, "expand")
    return node("nav", children, "navbar", "navbar-" + variant, "bg-" + background if background else "",
                expansion, **props)


@component
def NavbarBrand(*children, href=None, **props):
    return node("a" if href is not None else "span", children, "navbar-brand", attributes={"href": href}, **props)


@component
def Offcanvas(*children, title=None, id, placement="start", open=True, **props):
    identifier(id)
    choice(placement, ("start", "end", "top", "bottom"), "offcanvas placement")
    boolean(open, "open")
    title_id = id + "-title" if title is not None else None
    return node("section", (
        node("div", (node("h2", (title,), "offcanvas-title", id=title_id),), "offcanvas-header") if title is not None else None,
        node("div", children, "offcanvas-body"),
    ), "offcanvas", "offcanvas-" + placement, "fb-bs-offcanvas", "position-static", "show" if open else "",
        attributes={"id": id, "aria-labelledby": title_id, "hidden": not open}, **props)


@component
def Pagination(*children, label="Pagination", size=None, **props):
    choice(size, SIZES, "pagination size", optional=True)
    return node("nav", (list_node("ul", children, "pagination", "pagination-" + size if size else ""),),
                attributes={"aria-label": label}, **props)


@component
def PageItem(*children, href=None, active=False, disabled=False, **props):
    boolean(active, "active")
    boolean(disabled, "disabled")
    link = node("a" if href is not None and not disabled and not active else "span", children, "page-link",
                attributes={"href": href if not disabled and not active else None})
    link = link_box(link, link.children, href if not disabled and not active else None)
    return list_node("li", (link,), "page-item", {"active": active, "disabled": disabled},
                attributes={"aria-current": "page" if active else None, "aria-disabled": "true" if disabled else None}, **props)


@component
def Popover(*children, title=None, **props):
    return node("aside", (
        node("h3", (title,), "popover-header") if title is not None else None,
        node("div", children, "popover-body"),
    ), "popover", "position-static", **props)


@component
def Tooltip(*children, **props):
    return node("aside", (node("div", children, "tooltip-inner"),), "tooltip", "show", "position-static", **props)


@component
def Progress(*children, **props):
    return node("div", children, "progress", **props)


@component
def ProgressBar(value, *, max=100, label=None, variant="primary", striped=False, **props):
    choice(variant, COLORS, "progress variant")
    boolean(striped, "striped")
    if any(isinstance(number, bool) or not isinstance(number, (int, float, Decimal)) or not math.isfinite(number)
           for number in (value, max)) or max <= 0 or not 0 <= value <= max:
        raise AuthoringError("Progress requires finite numbers and 0 <= value <= max, with max > 0", code="bootstrap.progress")
    props["style"] = Style(props.pop("style", None), width=pct(float(value) / float(max) * 100))
    return node("div", (label,), "progress-bar", "bg-" + variant, "progress-bar-striped" if striped else "",
                attributes={"role": "progressbar", "aria-valuenow": value, "aria-valuemin": 0, "aria-valuemax": max,
                            "aria-label": label if isinstance(label, str) else None}, **props)


@component
def Scrollspy(*children, navigation, **props):
    """A static navigation/content composition; mark the current NavLink explicitly."""
    return node("div", (navigation, node("div", children)), "fb-bs-scrollspy", **props)


@component
def Spinner(*, label="Loading", variant="primary", grow=False, small=False, **props):
    choice(variant, COLORS, "spinner variant")
    boolean(grow, "grow")
    boolean(small, "small")
    family = "spinner-grow" if grow else "spinner-border"
    result = node("span", (node("span", (label,), "visually-hidden"),), family, "fb-bs-spinner",
                family + "-sm" if small else "", "text-" + variant, attributes={"role": "img", "aria-label": label}, **props)
    colors = dict(zip(COLORS, ("#0d6efd", "#6c757d", "#198754", "#dc3545", "#ffc107", "#0dcaf0", "#f8f9fa", "#212529")))
    appearance = Style(background_color=colors[variant]) if grow else Style(border_color=colors[variant], border_right=".25em solid transparent")
    return replace(result, _print=node("span", result.children, style=appearance, attributes={"role": "img"}))


@component
def Toast(*children, title=None, subtitle=None, image=None, open=True, **props):
    boolean(open, "open")
    header = node("div", (image, node("strong", (title,), "me-auto"),
                         node("small", (subtitle,), "ms-2") if subtitle is not None else None), "toast-header") if title is not None else None
    return node("div", (header, node("div", children, "toast-body")), "toast", "show" if open else "hide",
                attributes={"hidden": not open}, **props)


@dataclass(frozen=True, init=False)
class CarouselItem:
    children: tuple
    caption: tuple
    key: str | None

    def __init__(self, *children, caption=None, key=None):
        if key is not None:
            identifier(key)
        object.__setattr__(self, "children", _children(children))
        object.__setattr__(self, "caption", _children((caption,)))
        object.__setattr__(self, "key", key)


@component
def Carousel(*items, active=0, dark=False, **props):
    boolean(dark, "dark")
    items = _items(items, CarouselItem)
    active = _active(items, active)
    slides = [node("div", (*item.children, node("div", item.caption, "carousel-caption") if item.caption else None),
                   "carousel-item", "active" if key == active else "", attributes={"hidden": key != active})
              for key, item in items]
    return node("div", (node("div", slides, "carousel-inner"),), "carousel", "carousel-dark" if dark else "", **props)
