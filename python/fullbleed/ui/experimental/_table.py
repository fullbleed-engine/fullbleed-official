# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Callable, Mapping, Set
from dataclasses import dataclass
from typing import Generic, TypeVar

from ._style import AuthoringError, Length, Style, pt
from ._tree import Node, _element, component
from ._schema import COLUMN_ALIGNMENTS, ROW_BREAKS, TABLE_PARTS

T = TypeVar("T")


@dataclass(frozen=True)
class Column(Generic[T]):
    """A header and cell callback with optional explicit layout choices.

    Omitted/None alignment uses the table's left-aligned default and remains
    customizable by selector styles. An explicit alignment or width is authored
    on both the header and cells; part overrides can replace it.
    """

    label: str
    cell: Callable[[T], object]
    align: str | None = None
    width: Length | None = None

    def __post_init__(self):
        if not isinstance(self.label, str) or not self.label.strip() or not callable(self.cell):
            raise AuthoringError("Column requires a label and a cell function", code="table.column")
        if self.align is not None and self.align not in COLUMN_ALIGNMENTS:
            raise AuthoringError("Column alignment must be left, right, center, or None", code="table.align")
        if self.width is not None and (not isinstance(self.width, Length) or self.width.value <= 0):
            raise AuthoringError("Column width requires a positive length", code="table.width")


@component(parts=TABLE_PARTS[:1], optional_parts=TABLE_PARTS[1:])
def DataTable(rows, *, columns, caption=None, empty="No records.", repeat_header=True,
              row_break=None, style=None, class_name=None, attrs=None) -> Node:
    """Capture rows and cell callbacks once, with repeated semantic column headers.

    Rows prefer intact placement by default. None leaves that preference to
    defaults or selector styles; explicit row_break='avoid' or 'auto' overrides
    normal selectors. Top- or baseline-aligned oversized text cells can
    continue across pages. Unsupported layouts raise layout.unsplittable when
    the native trace reports forced overflow; this is not a general validator.
    Use with_parts(header=..., cell=..., row=..., caption=..., empty=...) to customize.
    """
    if isinstance(columns, (Mapping, Set, str, bytes)):
        raise AuthoringError("Table columns must be ordered Column values", code="table.columns")
    columns = tuple(columns)
    if not columns or any(not isinstance(column, Column) for column in columns):
        raise AuthoringError("DataTable requires at least one Column", code="table.columns")
    if not isinstance(repeat_header, bool) or row_break is not None and row_break not in ROW_BREAKS:
        raise AuthoringError("Use boolean repeat_header and row_break='avoid', 'auto', or None", code="table.pagination")
    if isinstance(rows, (Mapping, Set, str, bytes, bytearray)):
        raise AuthoringError("Table rows must be an ordered iterable", code="table.rows")
    def located(node, ordinal, label):
        # Virtual scopes preserve the printed label and add an unambiguous
        # column position without extra HTML boxes or changing native node IDs.
        return Node(None, (Node(None, (node,), name=label),), name=f"column {ordinal}")

    headers = [located(_element("th", (column.label,), attrs={"scope": "col"}, part="header",
                        base=Style(text_align="left", padding=(pt(8), pt(6)),
                                   border_bottom="1pt solid #667580", font_weight=700),
                        style=Style(text_align=column.align, width=column.width)), ordinal, column.label)
               for ordinal, column in enumerate(columns, 1)]
    rendered = []
    for index, row in enumerate(rows, 1):
        cells = []
        for ordinal, column in enumerate(columns, 1):
            path = (f"row {index}", f"column {ordinal}", column.label)
            try:
                cell = _element("td", (column.cell(row),), part="cell",
                                      base=Style(text_align="left", vertical_align="top",
                                                 padding=(pt(8), pt(6)), border_bottom="0.5pt solid #d7dfe5"),
                                      style=Style(text_align=column.align, width=column.width))
                cells.append(located(cell, ordinal, column.label))
            except AuthoringError as error:
                raise AuthoringError(error.message, code=error.code, path=(*path, *error.path)) from error
            except Exception as error:
                raise AuthoringError(f"Cell function failed: {error}", code="table.cell", path=path) from error
        rendered.append(Node(None, (_element("tr", cells, part="row", base=Style(break_inside="avoid"),
                                            style=Style(break_inside=row_break)),), name=f"row {index}"))
    if not rendered:
        rendered.append(_element("tr", (_element("td", (empty,), attrs={"colspan": len(columns)}, part="empty",
                                                   base=Style(padding=pt(12))),)))
    children = []
    if caption is not None:
        children.append(_element("caption", (caption,), part="caption", base=Style(text_align="left", padding_bottom=pt(8))))
    header = _element("tr", headers, base=Style(break_after="avoid"))
    if repeat_header:
        children.extend((_element("thead", (header,)), _element("tbody", rendered)))
    else:
        # The native table parser treats a literal thead as a repeated group,
        # regardless of its display declaration. Keep scoped column-header cells
        # in the body for a heading that appears only once.
        children.append(_element("tbody", (header, *rendered)))
    table = _element("table", children, base=Style(width="100%", border_collapse="collapse", table_layout="fixed"),
                     style=style, class_name=class_name, attrs=attrs)
    return table
