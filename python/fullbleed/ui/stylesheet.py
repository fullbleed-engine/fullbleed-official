"""Reusable document styles authored as Python mappings or explicit CSS."""
from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from .style import style_to_css


class StyleSheet:
    """Ordered CSS rules. Later rules participate in the normal CSS cascade.

    Mapping keys are selectors (or declaration rules such as ``@page``).
    Values use the same style composition as an element's ``style=`` prop.
    Use a CSS string for nested at-rules and other advanced CSS syntax.
    """

    def __init__(self, *fragments: Any):
        self._fragments: list[str] = []
        self.add(*fragments)

    def add(self, *fragments: Any) -> StyleSheet:
        for fragment in fragments:
            if fragment is None or fragment is False:
                continue
            if isinstance(fragment, StyleSheet):
                self._fragments.append(fragment.to_css())
            elif isinstance(fragment, str):
                self._fragments.append(fragment.strip())
            elif isinstance(fragment, Mapping):
                for selector, declarations in fragment.items():
                    if not isinstance(selector, str) or not selector.strip():
                        raise ValueError("Stylesheet selectors must be nonempty strings")
                    if isinstance(declarations, Mapping) and any(isinstance(value, Mapping) for value in declarations.values()):
                        raise TypeError("Nested stylesheet rules require an explicit CSS string")
                    self._fragments.append(f"{selector.strip()} {{ {style_to_css(declarations)} }}")
            else:
                raise TypeError("Stylesheet fragments must be mappings, CSS strings or StyleSheet values")
        return self

    def to_css(self) -> str:
        return "\n".join(fragment for fragment in self._fragments if fragment)


def component_styles() -> str:
    """Small layout defaults used by DocumentArtifact's rendering methods.

    No fonts, external assets, print dimensions or design theme are loaded.
    Authored styles follow these rules and can override them normally.
    """
    return StyleSheet({
        ".ui-stack": {"display": "flex", "flex_direction": "column", "gap": "8pt"},
        ".ui-row": {"display": "flex", "align_items": "center", "justify_content": "space-between", "gap": "8pt"},
        ".ui-layout-grid": {"display": "grid", "grid_template_columns": "1fr 1fr", "gap": "8pt"},
        ".ui-inline": {"display": "inline-flex", "align_items": "center", "gap": "4pt"},
        ".ui-center": {"display": "flex", "flex_direction": "column", "align_items": "center", "text_align": "center"},
        ".ui-card": {"border": "1px solid #d3ded8", "border_radius": "4pt", "padding": "12pt"},
        ".ui-table": {"width": "100%", "border_collapse": "collapse"},
    }).to_css()
