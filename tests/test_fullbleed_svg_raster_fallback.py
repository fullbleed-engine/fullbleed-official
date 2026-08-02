from __future__ import annotations

import zlib

import pytest

import fullbleed


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def _require_svg_raster_feature() -> None:
    _require_pdf_engine()
    features = dict(fullbleed.build_features())
    if not features.get("svg_raster", False):
        pytest.skip("svg_raster compatibility feature is not enabled")


def _decode_png_rgba(data: bytes) -> tuple[int, int, list[bytes]]:
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError("expected PNG bytes")

    width = height = color_type = bit_depth = None
    idat = bytearray()
    pos = 8
    while pos + 8 <= len(data):
        length = int.from_bytes(data[pos : pos + 4], "big")
        ctype = data[pos + 4 : pos + 8]
        pos += 8
        chunk = data[pos : pos + length]
        pos += length + 4
        if ctype == b"IHDR":
            width = int.from_bytes(chunk[0:4], "big")
            height = int.from_bytes(chunk[4:8], "big")
            bit_depth = chunk[8]
            color_type = chunk[9]
        elif ctype == b"IDAT":
            idat.extend(chunk)
        elif ctype == b"IEND":
            break

    assert width is not None and height is not None and bit_depth == 8
    assert color_type in (2, 6)
    bpp = 4 if color_type == 6 else 3
    raw = zlib.decompress(bytes(idat))
    stride = width * bpp
    prev = bytearray(stride)
    rows: list[bytes] = []
    off = 0
    for _ in range(height):
        filter_type = raw[off]
        off += 1
        row = bytearray(raw[off : off + stride])
        off += stride

        if filter_type == 1:
            for i in range(stride):
                row[i] = (row[i] + (row[i - bpp] if i >= bpp else 0)) & 255
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 255
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + ((left + prev[i]) // 2)) & 255
        elif filter_type == 4:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                up = prev[i]
                up_left = prev[i - bpp] if i >= bpp else 0
                p = left + up - up_left
                pa = abs(p - left)
                pb = abs(p - up)
                pc = abs(p - up_left)
                pred = left if pa <= pb and pa <= pc else up if pb <= pc else up_left
                row[i] = (row[i] + pred) & 255
        elif filter_type != 0:
            raise AssertionError(f"unsupported PNG filter type {filter_type}")

        if bpp == 3:
            rgba = bytearray(width * 4)
            for x in range(width):
                rgba[x * 4 : x * 4 + 4] = row[x * 3 : x * 3 + 3] + b"\xff"
            rows.append(bytes(rgba))
        else:
            rows.append(bytes(row))
        prev = row

    return width, height, rows


def test_svg_text_renders_through_native_vector_path() -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(svg_raster_fallback=False)
    html = (
        "<!doctype html><html><body>"
        "<svg width='160' height='64' viewBox='0 0 160 64' aria-label='SVG text'>"
        "<text x='8' y='46' font-family='Arial, sans-serif' font-size='44' fill='#cc0000'>SVG</text>"
        "</svg>"
        "</body></html>"
    )
    css = "@page { size: 2.5in 1in; margin: 0; } body { margin: 0; background: white; } svg { display: block; }"

    pages = list(engine.render_image_pages(html, css, 144))

    assert len(pages) == 1
    width, height, rows = _decode_png_rgba(pages[0])
    red_pixels = 0
    for y in range(height):
        row = rows[y]
        for x in range(width):
            r, g, b, a = row[x * 4 : x * 4 + 4]
            if a > 200 and r > 150 and g < 90 and b < 90:
                red_pixels += 1

    assert red_pixels > 100


def test_dependency_free_svg_fallback_renders_patterns_masks_and_filters() -> None:
    _require_svg_raster_feature()

    engine = fullbleed.PdfEngine(svg_raster_fallback=True)
    html = (
        "<!doctype html><html><body>"
        "<svg width='160' height='64' viewBox='0 0 160 64'>"
        "<defs>"
        "<pattern id='p' patternUnits='userSpaceOnUse' width='10' height='10'>"
        "<rect width='5' height='5' fill='#cc0000'/><rect x='5' y='5' width='5' height='5' fill='#cc0000'/>"
        "</pattern>"
        "<mask id='m'><rect x='56' y='4' width='48' height='48' fill='white'/>"
        "<circle cx='80' cy='28' r='9' fill='black'/></mask>"
        "<filter id='b'><feGaussianBlur stdDeviation='2.5'/></filter>"
        "</defs>"
        "<rect x='2' y='4' width='48' height='48' fill='url(#p)'/>"
        "<rect x='56' y='4' width='48' height='48' fill='#0044cc' mask='url(#m)'/>"
        "<rect x='118' y='14' width='28' height='28' fill='#008844' filter='url(#b)'/>"
        "</svg></body></html>"
    )
    css = "@page { size: 2.5in 1in; margin: 0; } body { margin: 0; background: white; } svg { display: block; }"

    page = list(engine.render_image_pages(html, css, 144))[0]
    width, height, rows = _decode_png_rgba(page)
    red = blue = green = 0
    for row in rows:
        for x in range(width):
            r, g, b, a = row[x * 4 : x * 4 + 4]
            if a > 200 and r > 140 and g < 90 and b < 90:
                red += 1
            if a > 200 and b > 140 and r < 90 and g < 120:
                blue += 1
            if a > 200 and g > 70 and r < 90 and b < 110:
                green += 1

    assert red > 1_000
    assert blue > 1_500
    assert green > 500
    hole = rows[42][120 * 4 : 120 * 4 + 4]
    assert all(channel > 245 for channel in hole[:3])


def test_dependency_free_svg_fallback_renders_auto_oriented_markers() -> None:
    _require_svg_raster_feature()

    engine = fullbleed.PdfEngine(svg_raster_fallback=True)
    html = (
        "<!doctype html><html><body>"
        "<svg width='120' height='64' viewBox='0 0 120 64'>"
        "<defs><marker id='arrow' markerWidth='8' markerHeight='8' refX='8' refY='4' "
        "markerUnits='userSpaceOnUse' orient='auto' viewBox='0 0 8 8'>"
        "<path d='M0 0 L8 4 L0 8 Z' fill='#cc0000'/></marker></defs>"
        "<polyline points='10,48 56,14 108,44' fill='none' stroke='#003399' stroke-width='3' "
        "marker-start='url(#arrow)' marker-mid='url(#arrow)' marker-end='url(#arrow)'/>"
        "</svg></body></html>"
    )
    css = "@page { size: 2.5in 1in; margin: 0; } body { margin: 0; background: white; } svg { display: block; }"

    page = list(engine.render_image_pages(html, css, 144))[0]
    width, _, rows = _decode_png_rgba(page)
    red = blue = 0
    for row in rows:
        for x in range(width):
            r, g, b, a = row[x * 4 : x * 4 + 4]
            if a > 200 and r > 140 and g < 90 and b < 90:
                red += 1
            if a > 200 and b > 100 and r < 80 and g < 120:
                blue += 1

    assert red > 80
    assert blue > 150
