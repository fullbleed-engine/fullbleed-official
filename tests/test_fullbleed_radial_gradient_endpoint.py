"""Final radial color stops must continue beyond the authored ending shape."""
from pathlib import Path
import runpy

import pytest
import fullbleed


decode_png = runpy.run_path(str(Path(__file__).resolve().parents[1] / "tools" / "run_css_fixture_suite.py"))["_decode_png_rgba_rows"]

CASES = [
    ("opaque", "circle 30pt at 50pt 50pt", "red 0%,gold 100%,blue 100%", "white", (0, 0, 255)),
    ("alpha", "circle 30pt at 50pt 50pt", "red 0%,gold 100%,rgba(0,0,255,.25) 100%", "white", (191, 191, 255)),
    ("alpha-only", "circle 30pt at 50pt 50pt", "red 0%,gold 100%,rgba(255,215,0,.25) 100%", "white", (255, 245, 191)),
    ("transparent", "circle 30pt at 50pt 50pt", "red 0%,gold 100%,rgba(0,0,255,0) 100%", "#00ff00", (0, 255, 0)),
    ("ellipse", "ellipse 30pt 15pt at 50pt 50pt", "red 0%,gold 100%,blue 100%", "white", (0, 0, 255)),
    ("offset", "circle 30pt at 20pt 25pt", "red 0%,gold 100%,blue 100%", "white", (0, 0, 255)),
    ("hard-band", "circle 30pt at 50pt 50pt", "red 0% 100%,blue 100%", "white", (0, 0, 255)),
    ("three-final-stops", "circle 30pt at 50pt 50pt", "red 0%,gold 100%,lime 100%,blue 100%", "white", (0, 0, 255)),
    ("rounded-endpoint", "circle 30pt at 50pt 50pt", "red 0%,gold 99.99999%,blue 100%", "white", (0, 0, 255)),
    ("early-final-stop", "circle 30pt at 50pt 50pt", "red 0%,gold 50%,blue 50%", "white", (0, 0, 255)),
    ("smooth-control", "circle 30pt at 50pt 50pt", "red,gold,blue", "white", (0, 0, 255)),
    ("repeating-control", "circle 30pt at 50pt 50pt", "red 0pt 10pt,blue 10pt 20pt", "white", (255, 0, 0)),
]


@pytest.mark.parametrize("name,shape,stops,backdrop,expected", CASES, ids=[row[0] for row in CASES])
def test_radial_final_stop_continues_outward(tmp_path, name, shape, stops, backdrop, expected):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    engine = fullbleed.PdfEngine()
    kind = "repeating-radial-gradient" if name == "repeating-control" else "radial-gradient"
    html = "<!doctype html><html><body><div></div></body></html>"
    css = ("@page{size:100pt 100pt;margin:0}html,body{margin:0}body{background:" + backdrop + "}"
           "div{width:100pt;height:100pt;background:" + kind + "(" + shape + "," + stops + ")}")
    direct = bytes(engine.render_pdf(html, css))
    assert direct == bytes(engine.compile_pdf(html, css).render_pdf()) == bytes(engine.render_pdf(html, css))
    path = tmp_path / (name + ".pdf")
    path.write_bytes(direct)
    for dpi in (144, 300):
        pages = list(engine.render_finalized_pdf_image_pages(str(path), dpi))
        assert len(pages) == 1
        width, height, rows = decode_png(bytes(pages[0]))
        for x, y in ((95, 5), (95, 50), (95, 95)):
            px, py = int(x * dpi / 72), int(y * dpi / 72)
            assert px < width and py < height
            actual = tuple(rows[py][px * 4:px * 4 + 3])
            assert max(abs(a - b) for a, b in zip(actual, expected)) <= 1, (name, dpi, (x, y), actual, expected)
