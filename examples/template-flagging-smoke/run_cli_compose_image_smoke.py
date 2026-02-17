from __future__ import annotations

import json
import subprocess
import sys
import zlib
from pathlib import Path

import fullbleed


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
OUT.mkdir(parents=True, exist_ok=True)

TEMPLATE_PDF = OUT / "cli_image_template_blue.pdf"
INPUT_HTML = OUT / "cli_image_input.html"
INPUT_CSS = OUT / "cli_image_input.css"
BINDING_JSON = OUT / "cli_image_binding.json"
TEMPLATES_JSON = OUT / "cli_image_templates.json"
OUT_PDF = OUT / "cli_image_out.pdf"
OUT_IMG_DIR = OUT / "cli_image_pages"
REPORT_JSON = OUT / "cli_compose_image_smoke_report.json"


def _paeth_predictor(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa = abs(p - a)
    pb = abs(p - b)
    pc = abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def _read_png_pixel(path: Path, x: int, y: int) -> tuple[int, int, int, int]:
    data = path.read_bytes()
    if len(data) < 8 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError(f"not a PNG file: {path}")

    width = None
    height = None
    bit_depth = None
    color_type = None
    idat = bytearray()

    pos = 8
    while pos + 8 <= len(data):
        length = int.from_bytes(data[pos : pos + 4], "big")
        ctype = data[pos + 4 : pos + 8]
        pos += 8
        chunk = data[pos : pos + length]
        pos += length
        pos += 4  # CRC

        if ctype == b"IHDR":
            width = int.from_bytes(chunk[0:4], "big")
            height = int.from_bytes(chunk[4:8], "big")
            bit_depth = chunk[8]
            color_type = chunk[9]
        elif ctype == b"IDAT":
            idat.extend(chunk)
        elif ctype == b"IEND":
            break

    if width is None or height is None or bit_depth is None or color_type is None:
        raise RuntimeError(f"invalid PNG structure: {path}")
    if bit_depth != 8:
        raise RuntimeError(f"unsupported PNG bit depth: {bit_depth}")
    if color_type == 2:
        bpp = 3
    elif color_type == 6:
        bpp = 4
    else:
        raise RuntimeError(f"unsupported PNG color type: {color_type}")
    if x < 0 or y < 0 or x >= width or y >= height:
        raise RuntimeError(f"pixel out of bounds ({x},{y}) for PNG {width}x{height}")

    raw = zlib.decompress(bytes(idat))
    stride = width * bpp
    expected = (stride + 1) * height
    if len(raw) < expected:
        raise RuntimeError(
            f"truncated PNG payload for {path}: expected at least {expected}, got {len(raw)}"
        )

    prev = bytearray(stride)
    off = 0
    for row_idx in range(height):
        filter_type = raw[off]
        off += 1
        row = bytearray(raw[off : off + stride])
        off += stride

        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + left) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                up = prev[i]
                row[i] = (row[i] + ((left + up) // 2)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                up = prev[i]
                up_left = prev[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + _paeth_predictor(left, up, up_left)) & 0xFF
        else:
            raise RuntimeError(f"unsupported PNG filter type: {filter_type}")

        if row_idx == y:
            idx = x * bpp
            if bpp == 3:
                return row[idx], row[idx + 1], row[idx + 2], 255
            return row[idx], row[idx + 1], row[idx + 2], row[idx + 3]

        prev = row

    raise RuntimeError(f"pixel row not found for y={y}")


def build_template(path: Path) -> None:
    html = """
<!doctype html>
<html><body><section class="tpl"><p>BLUE TEMPLATE</p></section></body></html>
""".strip()
    css = """
@page { size: 8.5in 11in; margin: 0; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; }
.tpl { width: 8.5in; height: 11in; background: rgb(0,0,255); box-sizing: border-box; padding: 18pt; }
p { margin: 0; font-size: 10pt; color: #fff; }
""".strip()
    engine = fullbleed.PdfEngine(page_width="8.5in", page_height="11in", margin="0pt")
    engine.render_pdf_to_file(html, css, str(path))


def write_inputs() -> None:
    html = """
<!doctype html>
<html><body>
<section class="p"><div data-fb="fb.feature.blue=1"></div><p>Page 1</p></section>
<section class="p"><div data-fb="fb.feature.blue=1"></div><p>Page 2</p></section>
</body></html>
""".strip()
    css = """
@page { size: 8.5in 11in; margin: 0.5in; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; color: #111; }
.p:not(:last-child) { break-after: page; }
p { margin: 0; }
""".strip()
    binding = {
        "default_template_id": "tpl-blue",
        "by_feature": {"blue": "tpl-blue"},
        "feature_prefix": "fb.feature.",
    }
    templates = [
        {
            "template_id": "tpl-blue",
            "pdf_path": str(TEMPLATE_PDF),
        }
    ]

    INPUT_HTML.write_text(html, encoding="utf-8")
    INPUT_CSS.write_text(css, encoding="utf-8")
    BINDING_JSON.write_text(json.dumps(binding, ensure_ascii=True, indent=2), encoding="utf-8")
    TEMPLATES_JSON.write_text(
        json.dumps(templates, ensure_ascii=True, indent=2), encoding="utf-8"
    )


def render_with_cli() -> dict:
    args = [
        sys.executable,
        "-m",
        "fullbleed_cli.cli",
        "--json",
        "render",
        "--html",
        str(INPUT_HTML),
        "--css",
        str(INPUT_CSS),
        "--template-binding",
        str(BINDING_JSON),
        "--templates",
        str(TEMPLATES_JSON),
        "--emit-image",
        str(OUT_IMG_DIR),
        "--out",
        str(OUT_PDF),
    ]
    proc = subprocess.run(args, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise RuntimeError(
            f"cli render failed rc={proc.returncode}: {proc.stdout.strip()} {proc.stderr.strip()}"
        )
    try:
        payload = json.loads(proc.stdout.strip())
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"cli returned non-json output: {proc.stdout!r}") from exc
    return payload


def validate(payload: dict) -> dict:
    if not payload.get("ok", False):
        raise RuntimeError(f"render payload not ok: {payload}")
    outputs = payload.get("outputs") or {}
    image_paths = outputs.get("image_paths") or []
    image_mode = outputs.get("image_mode")
    compose = outputs.get("template_compose") or {}
    pages_written = int(compose.get("pages_written") or 0)

    if image_mode != "composed_pdf":
        raise RuntimeError(f"expected image_mode=composed_pdf, got: {image_mode!r}")
    if pages_written <= 0:
        raise RuntimeError(f"invalid pages_written from compose payload: {pages_written}")
    if len(image_paths) != pages_written:
        raise RuntimeError(
            f"image/page mismatch: images={len(image_paths)} pages_written={pages_written}"
        )

    first_png = Path(image_paths[0])
    if not first_png.exists():
        raise RuntimeError(f"first image artifact missing: {first_png}")
    sample = _read_png_pixel(first_png, 10, 10)
    # Expect template-blue background near top-left.
    is_blue = (
        isinstance(sample, tuple)
        and len(sample) >= 3
        and int(sample[2]) >= 200
        and int(sample[0]) <= 40
        and int(sample[1]) <= 40
    )
    if not is_blue:
        raise RuntimeError(f"expected blue template background in first image, got pixel={sample}")

    return {
        "ok": True,
        "image_mode": image_mode,
        "pages_written": pages_written,
        "image_count": len(image_paths),
        "first_image": str(first_png),
        "first_pixel_10_10": list(sample),
    }


def main() -> None:
    build_template(TEMPLATE_PDF)
    write_inputs()
    payload = render_with_cli()
    result = validate(payload)
    report = {
        "schema": "fullbleed.template_flagging_cli_compose_image_smoke.v1",
        "ok": True,
        "render": payload,
        "validation": result,
        "paths": {
            "template_pdf": str(TEMPLATE_PDF),
            "out_pdf": str(OUT_PDF),
            "out_image_dir": str(OUT_IMG_DIR),
        },
    }
    REPORT_JSON.write_text(json.dumps(report, ensure_ascii=True, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))


if __name__ == "__main__":
    main()
