# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Mapping
from dataclasses import replace

from .._style import AuthoringError
from .._table import Column, DataTable as _DataTable
from .._tree import component
from ._core import boolean, choice, classes, node, resources
from ._schema import BREAKPOINTS, COLORS
from ._utilities import utilities


@component
def Container(*children, fluid=False, **props):
    if fluid is True:
        name = "container-fluid"
    elif fluid is False:
        name = "container"
    else:
        name = "container-" + choice(fluid, BREAKPOINTS, "fluid")
    return node("div", children, name, **props)


@component
def Row(*children, columns=None, gutter=None, gutter_x=None, gutter_y=None, **props):
    return node("div", children, "row", utilities(row_cols=columns, g=gutter, gx=gutter_x, gy=gutter_y), **props)


def _column_classes(span):
    if span is None:
        return "col"
    values = span if isinstance(span, Mapping) else {"xs": span}
    result = []
    for breakpoint, value in values.items():
        choice(breakpoint, ("xs", "default", *BREAKPOINTS), "column breakpoint")
        if isinstance(value, bool) or not (value == "auto" or isinstance(value, int) and 1 <= value <= 12):
            raise AuthoringError("Column spans must be 1..12 or 'auto'", code="bootstrap.column")
        result.append("col-" + ("" if breakpoint in {"xs", "default"} else breakpoint + "-") + str(value))
    return classes(result) or "col"


@component
def Col(*children, span=None, offset=None, order=None, **props):
    return node("div", children, _column_classes(span), utilities(offset=offset, order=order), **props)


@component
def Heading(*children, level=2, display=None, **props):
    if isinstance(level, bool) or not isinstance(level, int) or not 1 <= level <= 6:
        raise AuthoringError("Heading level must be 1..6", code="heading.level")
    return node("h" + str(level), children, utilities(display=display), **props)


@component
def Link(*children, href, color=None, stretched=False, **props):
    choice(color, COLORS, "link color", optional=True)
    boolean(stretched, "stretched")
    return node("a", children, "link-" + color if color else "", "stretched-link" if stretched else "",
                attributes={"href": href}, **props)


@component
def Image(*, src, alt, fluid=True, thumbnail=False, rounded=False, **props):
    if not isinstance(src, str) or not src or not isinstance(alt, str):
        raise AuthoringError("Image needs src and an explicit text alternative", code="image.required")
    for label, flag in (("fluid", fluid), ("thumbnail", thumbnail), ("rounded", rounded)):
        boolean(flag, label)
    return node("img", (), "img-fluid" if fluid else "", "img-thumbnail" if thumbnail else "",
                "rounded" if rounded else "", attributes={"src": src, "alt": alt}, **props)


@component
def FigureImage(*, src, alt, **props):
    props["class_name"] = classes("figure-img", props.pop("class_name", None))
    return Image(src=src, alt=alt, **props)


def table_classes(*, variant=None, striped=False, hover=False, bordered=False, borderless=False, small=False):
    choice(variant, COLORS, "table variant", optional=True)
    if bordered and borderless:
        raise AuthoringError("Choose bordered or borderless", code="bootstrap.table")
    return classes("table", "table-" + variant if variant else "",
                   {"table-striped": striped, "table-hover": hover, "table-bordered": bordered,
                    "table-borderless": borderless, "table-sm": small})


@component
def Table(*children, variant=None, striped=False, hover=False, bordered=False, borderless=False,
          small=False, responsive=False, caption_top=False, **props):
    boolean(caption_top, "caption_top")
    table = node("table", children, table_classes(variant=variant, striped=striped, hover=hover,
                 bordered=bordered, borderless=borderless, small=small), "caption-top" if caption_top else "", **props)
    if responsive is False:
        return table
    suffix = "" if responsive is True else "-" + choice(responsive, BREAKPOINTS, "responsive")
    return node("div", (table,), "table-responsive" + suffix)


@component
def DataTable(rows, *, columns, caption=None, variant=None, striped=False, hover=False, bordered=False,
              borderless=False, small=False, empty="No records.", repeat_header=True, row_break=None, **props):
    class_name = classes(table_classes(variant=variant, striped=striped, hover=hover, bordered=bordered,
                                      borderless=borderless, small=small), props.pop("class_name", None))
    if "id" in props:
        attrs = dict(props.pop("attrs", None) or {})
        if "id" in attrs:
            raise AuthoringError("Supply id once", code="bootstrap.id")
        attrs["id"] = props.pop("id")
        props["attrs"] = attrs
    table = _DataTable(rows, columns=columns, caption=caption, empty=empty, repeat_header=repeat_header,
                       row_break=row_break, class_name=class_name, **props)
    return replace(table, _resources=resources())


@component
def Ratio(*children, ratio="16x9", **props):
    choice(ratio, ("1x1", "4x3", "16x9", "21x9"), "ratio")
    return node("div", children, "ratio", "ratio-" + ratio, **props)


@component
def VisuallyHidden(*children, focusable=False, **props):
    boolean(focusable, "focusable")
    return node("span", children, "visually-hidden-focusable" if focusable else "visually-hidden", **props)
