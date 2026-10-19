# SPDX-License-Identifier: MIT
"""Small shared components for the two designed showcase documents."""
from __future__ import annotations

from decimal import Decimal
from pathlib import Path
import xml.etree.ElementTree as ET

from fullbleed.ui.experimental import (
    Document, Footer, Fragment, Node, Page, Text, component, mm,
)

ROOT = Path(__file__).resolve().parent


def money(value) -> str:
    return f"${Decimal(value):,.2f}"


@component
def Lines(*lines) -> Node:
    """Compose deliberate print line breaks, including styled inline children."""
    return Fragment(*(child for i, line in enumerate(lines)
                      for child in ((Node("br"), line) if i else (line,))))


@component
def Folio(label: str, page: str) -> Node:
    return Footer(Text(label), Text(page), class_name="footer")


@component
def VectorArtwork(filename: str, *, class_name: str) -> Node:
    """Reuse a trusted local SVG asset as vector nodes, without HTML injection."""
    root = ET.parse(ROOT / filename).getroot()

    def node(element):
        children = [element.text] if element.text else []
        for child in element:
            children.append(node(child))
            if child.tail:
                children.append(child.tail)
        attrs = dict(element.attrib)
        if element is root:
            attrs.update({"xmlns": "http://www.w3.org/2000/svg", "class": class_name})
        return Node(element.tag.removeprefix("{http://www.w3.org/2000/svg}"),
                    tuple(children), tuple(attrs.items()))

    return node(root)


def design_document(*children, kind: str, title: str) -> Document:
    # Both authoring paths deliberately share the original art direction.
    # Page owns paper geometry in the component API; selector CSS owns the rest.
    common = (ROOT / "common.css").read_text(encoding="utf-8")
    page_rule = "@page { size: A4; margin: 0; }"
    if not common.startswith(page_rule):
        raise ValueError("Update the explicit Page geometry when common.css changes")
    styles = common[len(page_rule):] + "\n" + (ROOT / f"{kind}.css").read_text(encoding="utf-8")
    # DataTable owns its cells. These two designs put numeric values after the
    # first column; mirror the original HTML's .number wrapping rule there.
    styles += "\ntable th:not(:first-child), table td:not(:first-child) { white-space: nowrap; }"
    return Document(*children, title=title, lang="en-US", page=Page(size="A4", margin=mm(0)),
                    styles=styles)
