from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

import fullbleed
from fullbleed_cli import cli


def _insights(tmp_path: Path, pages: list[dict]) -> dict:
    log = tmp_path / "bounds.jsonl"
    log.write_text(json.dumps({
        "type": "jit.docplan", "page_size": {"w": 612, "h": 792}, "pages": pages,
    }) + "\n", encoding="utf-8")
    return cli._collect_jit_insights(log)


def test_overflow_uses_each_named_page_size_and_preserves_legacy_fallback(tmp_path: Path) -> None:
    placement = {"bbox": {"x": 120, "y": 120, "w": 10, "h": 10}}
    result = _insights(tmp_path, [
        {"n": 1, "placements": [placement]},
        {"n": 2, "page_size": {"w": 100, "h": 100}, "placements": [placement]},
        {"n": 3, "page_size": {"w": 1000, "h": 1000}, "placements": [
            {"bbox": {"x": 800, "y": 800, "w": 10, "h": 10}},
        ]},
    ])
    assert result["overflow_signal"] is True
    assert result["overflow_count"] == 1
    assert result["overflow_samples"][0]["page"] == 2
    assert result["overflow_samples"][0]["page_size"] == {"w": 100.0, "h": 100.0}


@pytest.mark.parametrize("size", [None, {}, {"w": 0, "h": 100}, {"w": "nan", "h": 100}, {"w": 100, "h": "inf"}])
def test_invalid_page_geometry_cannot_pass_strict_overflow_verification(tmp_path: Path, size: object) -> None:
    result = _insights(tmp_path, [{"n": 1, "page_size": size, "placements": []}])
    assert result["overflow_signal"] is False
    failures = cli._evaluate_failures(SimpleNamespace(fail_on=["overflow"]), 0, [], result)
    assert [failure["code"] for failure in failures] == ["OVERFLOW_SIGNAL_UNAVAILABLE"]


@pytest.mark.parametrize("bounds", [
    {"x": "nan", "y": 10, "w": 20, "h": 20},
    {"x": 10, "y": 10, "w": -20, "h": 20},
    {"x": 10, "y": 10, "w": 20, "h": "inf"},
])
def test_invalid_placement_geometry_cannot_pass_strict_overflow_verification(tmp_path: Path, bounds: dict) -> None:
    result = _insights(tmp_path, [{"n": 1, "placements": [{"bbox": bounds}]}])
    assert result["overflow_signal"] is False


@pytest.mark.parametrize("style", ["normal", "italic"])
def test_render_time_svg_text_trace_uses_the_same_page_basis_as_paint(style: str) -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("installed native engine required")
    font = Path(__file__).resolve().parents[1] / "python/fullbleed_assets/fonts/NotoSans-Regular.ttf"
    engine = fullbleed.PdfEngine(font_files=[str(font)], svg_form_xobjects=False)
    html = '<figure><svg width="400" height="100" viewBox="0 0 400 100"><text x="20" y="40" font-size="20">POSITION</text></svg></figure>'
    css = f"@page {{size:Letter; margin:36pt}} figure {{margin:0; font-family:'Noto Sans'; font-style:{style};}}"
    trace = engine.export_render_time_reading_order_trace(html, css)
    blocks = [block for page in trace["pages"] for block in page["blocks"] if block["text"] == "POSITION"]
    assert len(blocks) == 1
    assert blocks[0]["x"] == pytest.approx(51.0, abs=0.01)
    assert blocks[0]["y"] == pytest.approx(51.0, abs=0.01)
    assert blocks[0]["h"] == pytest.approx(15.0, abs=0.01)
