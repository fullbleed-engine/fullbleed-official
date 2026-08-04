from __future__ import annotations

import importlib.util
import io
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
ADAPTER_PATH = REPO_ROOT / "tools" / "ironpress_fullbleed_adapter.py"


def _load_adapter():
    spec = importlib.util.spec_from_file_location(
        "ironpress_fullbleed_adapter_test", ADAPTER_PATH
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_embedded_css_preserves_style_block_source_order() -> None:
    adapter = _load_adapter()

    css = adapter.embedded_css(
        "<head><STYLE>@page { size: 10px 10px; margin: 0 }"
        ".first { color: red }</STYLE>"
        "<style type='text/css'>.second { color: blue }</style></head>"
    )

    assert css.index(".first") < css.index(".second")


def test_embedded_css_requires_a_style_block() -> None:
    adapter = _load_adapter()

    with pytest.raises(RuntimeError, match="no embedded <style>"):
        adapter.embedded_css("<html><head></head><body></body></html>")


def test_embedded_css_requires_explicit_page_geometry() -> None:
    adapter = _load_adapter()

    with pytest.raises(RuntimeError, match="no explicit @page"):
        adapter.embedded_css("<html><head><style>body { margin: 0 }</style></head></html>")


def test_render_fixture_uses_parity_geometry_fonts_and_extracted_css(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    adapter = _load_adapter()
    base_path = tmp_path / "cases" / "probes"
    resource_root = tmp_path / "parity"
    base_path.mkdir(parents=True)
    font_root = resource_root / "fonts"
    font_root.mkdir(parents=True)
    for name in adapter.PARITY_FONT_NAMES:
        (font_root / name).write_bytes(b"test-font")

    observed: dict[str, object] = {}

    class FakeEngine:
        def __init__(self, **kwargs):
            observed["kwargs"] = kwargs

        def render_pdf(self, html: str, css: str) -> bytes:
            observed["html"] = html
            observed["css"] = css
            observed["cwd"] = Path.cwd()
            return b"%PDF-1.7\n%%EOF\n"

    monkeypatch.setitem(sys.modules, "fullbleed", SimpleNamespace(PdfEngine=FakeEngine))
    original_directory = Path.cwd()
    html = (
        "<html><head><style>@page { size: 10px 10px; margin: 0 }"
        ".box { width: 10px }</style></head></html>"
    )

    pdf = adapter.render_fixture(
        html,
        base_path=base_path,
        resource_root=resource_root,
    )

    assert pdf == b"%PDF-1.7\n%%EOF\n"
    assert observed["html"] == html
    assert observed["css"] == (
        "@page { size: 10px 10px; margin: 0 }.box { width: 10px }"
    )
    assert observed["cwd"] == base_path.resolve()
    assert Path.cwd() == original_directory
    expected_font_files = [
        str((font_root / name).resolve()) for name in adapter.PARITY_FONT_NAMES
    ]
    expected_font_files.extend(
        str(path)
        for path in (
            *adapter.PARITY_GENERIC_FONT_PATHS,
            *adapter.PARITY_FALLBACK_FONT_PATHS,
        )
        if path.is_file()
    )
    assert observed["kwargs"] == {"font_files": expected_font_files}


def test_server_reuses_engine_and_frames_individual_pdf_responses(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    adapter = _load_adapter()
    first_base = tmp_path / "cases" / "first"
    second_base = tmp_path / "cases" / "second"
    first_base.mkdir(parents=True)
    second_base.mkdir(parents=True)
    resource_root = tmp_path / "parity"
    font_root = resource_root / "fonts"
    font_root.mkdir(parents=True)
    for name in adapter.PARITY_FONT_NAMES:
        (font_root / name).write_bytes(b"test-font")

    constructed = 0
    rendered_from: list[Path] = []

    class FakeEngine:
        def __init__(self, **_kwargs):
            nonlocal constructed
            constructed += 1

        def render_pdf(self, html: str, _css: str) -> bytes:
            rendered_from.append(Path.cwd())
            marker = b"first" if "first" in html else b"second"
            return b"%PDF-1.7\n" + marker + b"\n%%EOF\n"

    monkeypatch.setitem(sys.modules, "fullbleed", SimpleNamespace(PdfEngine=FakeEngine))

    def request(base_path: Path, marker: str) -> bytes:
        base = str(base_path).encode()
        html = (
            f"<html><head><style>@page{{size:10px 10px;margin:0}}</style>"
            f"</head><body>{marker}</body></html>"
        ).encode()
        return adapter.REQUEST_HEADER.pack(len(base), len(html)) + base + html

    output = io.BytesIO()
    status = adapter.serve(
        resource_root,
        input_stream=io.BytesIO(
            request(first_base, "first") + request(second_base, "second")
        ),
        output_stream=output,
    )

    assert status == 0
    assert constructed == 1
    assert rendered_from == [first_base.resolve(), second_base.resolve()]
    response = io.BytesIO(output.getvalue())
    for marker in (b"first", b"second"):
        header = response.read(adapter.RESPONSE_HEADER.size)
        response_status, payload_length = adapter.RESPONSE_HEADER.unpack(header)
        payload = response.read(payload_length)
        assert response_status == 0
        assert marker in payload
    assert response.read() == b""
