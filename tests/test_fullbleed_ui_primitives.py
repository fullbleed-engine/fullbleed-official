from __future__ import annotations

from pathlib import Path

from fullbleed.ui import render_node
from fullbleed.ui.primitives import Spacer, Th


FIXTURE_DIR = Path(__file__).parent / "fixtures" / "fullbleed_ui"


def _fixture(name: str) -> str:
    return (FIXTURE_DIR / name).read_text(encoding="utf-8")


def test_spacer_snapshot() -> None:
    node = Spacer(block="0.75rem", inline="1.25rem")
    assert render_node(node) == _fixture("spacer.html")


def test_th_scope_emits_scope_attr() -> None:
    node = Th("Amount", scope="col")
    html = render_node(node)
    assert html == '<th scope="col" class="ui-th">Amount</th>'
