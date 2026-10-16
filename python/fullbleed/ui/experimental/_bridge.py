# SPDX-License-Identifier: MIT
"""Explicit migration into the existing Element/DocumentArtifact authoring API."""
from __future__ import annotations

from ..core import Element
from ._style import AuthoringError, Theme
from ._tree import Node


def to_legacy(node: Node, *, theme: Theme | None = None) -> tuple[Element | str, ...]:
    """Snapshot a component as children accepted by the existing fullbleed.ui API.

    Styles become inline declarations and tokens resolve using the supplied theme.
    The existing document still owns its CSS, typography, page geometry, metadata
    and renderer; Theme.text is a document setting, not copied onto each component.
    Fragment roots remain siblings, with no additional HTML wrapper or generated
    source IDs. The returned legacy Elements are independently mutable snapshots.
    """
    if not isinstance(node, Node):
        raise AuthoringError("to_legacy expects an experimental component node, not a document or HTML string",
                             code="migration.node")
    if theme is not None and not isinstance(theme, Theme):
        raise AuthoringError("to_legacy expects a Theme for token values", code="migration.theme")
    theme = theme or Theme()
    roots: list[Element | str] = []
    pending = [(node, (), roots)]
    while pending:
        item, path, parent = pending.pop()
        if isinstance(item, str):
            parent.append(item)
            continue
        if item.name is not None:
            path = (*path, item.name)
        target = parent
        if item.tag is not None:
            location = (*path, item.part or item.tag)
            try:
                declarations = item._effective_style().resolve(theme)
            except AuthoringError as error:
                raise AuthoringError(error.message, code=error.code, path=location) from error
            attributes = dict(item.attributes)
            if declarations:
                attributes["style"] = declarations
            element = Element(item.tag, props=attributes)
            parent.append(element)
            target = element.children
        pending.extend((child, path, target) for child in reversed(item.children))
    return tuple(roots)
