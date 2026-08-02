#!/usr/bin/env python3
"""Render IronPress parity fixtures through FullBleed.

The IronPress harness owns fixture selection, authenticated UA-style injection,
oracle rasterization, comparison, and reporting. This adapter is deliberately
limited to the candidate-renderer API boundary. It supports a one-shot stdin/PDF
stdout contract for diagnostics and a framed persistent-worker contract for the
parallel full corpus.
"""

from __future__ import annotations

import argparse
import os
import re
import struct
import sys
from pathlib import Path
from typing import BinaryIO, Sequence


PARITY_FONT_NAMES = ("ParitySans.ttf", "ParitySerif.ttf", "ParityMono.ttf")
STYLE_BLOCK = re.compile(
    r"<style\b[^>]*>(?P<css>.*?)</style\s*>",
    flags=re.IGNORECASE | re.DOTALL,
)
REQUEST_HEADER = struct.Struct(">IQ")
RESPONSE_HEADER = struct.Struct(">BQ")
MAX_BASE_PATH_BYTES = 1024 * 1024
MAX_HTML_BYTES = 64 * 1024 * 1024


def embedded_css(html: str) -> str:
    """Return embedded style blocks in their document source order."""

    blocks = [match.group("css") for match in STYLE_BLOCK.finditer(html)]
    if not blocks:
        raise RuntimeError("IronPress fixture contains no embedded <style> block")
    css = "\n".join(blocks)
    if "@page" not in css.lower():
        raise RuntimeError("IronPress fixture contains no explicit @page rule")
    return css


def parity_font_files(resource_root: Path) -> list[str]:
    font_root = resource_root / "fonts"
    paths = [font_root / name for name in PARITY_FONT_NAMES]
    missing = [path for path in paths if not path.is_file()]
    if missing:
        raise RuntimeError(f"missing IronPress parity font: {missing[0]}")
    return [str(path) for path in paths]


class FixtureRenderer:
    """Reusable FullBleed renderer with one deterministic font registry."""

    def __init__(self, resource_root: Path) -> None:
        try:
            import fullbleed
        except ImportError as error:
            raise RuntimeError(
                "fullbleed is not installed in the parity adapter environment"
            ) from error

        self.resource_root = resource_root.resolve(strict=True)
        if not self.resource_root.is_dir():
            raise RuntimeError(
                f"parity resource root is not a directory: {self.resource_root}"
            )
        # Every authenticated fixture supplies explicit @page geometry. Leaving
        # builder geometry unset allows that author rule to control the canvas;
        # explicit PdfEngine dimensions intentionally override CSS @page rules.
        self.engine = fullbleed.PdfEngine(
            font_files=parity_font_files(self.resource_root)
        )

    def render(self, html: str, *, base_path: Path) -> bytes:
        base_path = base_path.resolve(strict=True)
        if not base_path.is_dir():
            raise RuntimeError(f"fixture base path is not a directory: {base_path}")

        previous_directory = Path.cwd()
        try:
            # FullBleed resolves local url(...) inputs against the process working
            # directory. A persistent adapter is single-threaded, so changing its
            # process-local base between framed requests is deterministic.
            os.chdir(base_path)
            pdf = bytes(self.engine.render_pdf(html, embedded_css(html)))
        finally:
            os.chdir(previous_directory)

        if not pdf.startswith(b"%PDF-") or not pdf.rstrip().endswith(b"%%EOF"):
            raise RuntimeError("FullBleed returned a malformed PDF payload")
        return pdf


def render_fixture(
    html: str,
    *,
    base_path: Path,
    resource_root: Path,
) -> bytes:
    return FixtureRenderer(resource_root).render(html, base_path=base_path)


def _read_exact(stream: BinaryIO, length: int, *, allow_eof: bool = False) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    while remaining:
        chunk = stream.read(remaining)
        if not chunk:
            if allow_eof and not chunks:
                return b""
            raise RuntimeError("truncated FullBleed parity adapter request")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def serve(
    resource_root: Path,
    *,
    input_stream: BinaryIO | None = None,
    output_stream: BinaryIO | None = None,
) -> int:
    """Serve length-prefixed fixtures for one IronPress worker thread."""

    input_stream = input_stream or sys.stdin.buffer
    output_stream = output_stream or sys.stdout.buffer
    renderer = FixtureRenderer(resource_root)

    while True:
        header = _read_exact(input_stream, REQUEST_HEADER.size, allow_eof=True)
        if not header:
            return 0
        base_length, html_length = REQUEST_HEADER.unpack(header)
        if base_length == 0 or base_length > MAX_BASE_PATH_BYTES:
            raise RuntimeError(f"invalid fixture base-path length: {base_length}")
        if html_length == 0 or html_length > MAX_HTML_BYTES:
            raise RuntimeError(f"invalid fixture HTML length: {html_length}")

        base_bytes = _read_exact(input_stream, base_length)
        html_bytes = _read_exact(input_stream, html_length)
        try:
            base_path = Path(base_bytes.decode("utf-8"))
            html = html_bytes.decode("utf-8")
            payload = renderer.render(html, base_path=base_path)
            status = 0
        except (OSError, UnicodeError, RuntimeError, ValueError) as error:
            payload = str(error).encode("utf-8", errors="replace")
            status = 1

        output_stream.write(RESPONSE_HEADER.pack(status, len(payload)))
        output_stream.write(payload)
        output_stream.flush()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Render an IronPress parity fixture with FullBleed."
    )
    parser.add_argument("--server", action="store_true")
    parser.add_argument("--base-path", type=Path)
    parser.add_argument("--resource-root", type=Path, required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    try:
        if arguments.server:
            return serve(arguments.resource_root)
        if arguments.base_path is None:
            raise RuntimeError("--base-path is required outside --server mode")
        html = sys.stdin.buffer.read().decode("utf-8")
        if not html.strip():
            raise RuntimeError("fixture HTML on stdin is empty")
        pdf = render_fixture(
            html,
            base_path=arguments.base_path,
            resource_root=arguments.resource_root,
        )
        sys.stdout.buffer.write(pdf)
    except (OSError, UnicodeError, RuntimeError, ValueError) as error:
        print(f"fullbleed-parity-adapter: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
