from __future__ import annotations

import warnings

from fullbleed.ui import el, render_node, style
from fullbleed.ui.style import StyleWarning


def test_style_mapping_renders_inline_css_preserving_order() -> None:
    node = el("div", "x", style={"font_weight": 600, "color": "red"})
    assert render_node(node) == '<div style="font-weight: 600; color: red;">x</div>'


def test_style_fragments_merge_last_write_wins_with_order() -> None:
    node = el(
        "div",
        "x",
        style=[
            {"color": "red"},
            "font-weight: 600;",
            {"color": "blue"},
        ],
    )
    assert render_node(node) == '<div style="font-weight: 600; color: blue;">x</div>'


def test_style_bool_value_warns_and_is_skipped() -> None:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        css = style({"display": True, "color": "red"}).to_css()
    assert css == "color: red;"
    assert any(isinstance(w.message, StyleWarning) for w in caught)
