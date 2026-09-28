"""Replaced SVG viewport and CSS object-fit must be resolved only once."""
import base64

import pytest

import fullbleed
from test_fullbleed_svg_raster_fallback import _decode_png_rgba, _require_pdf_engine, _require_svg_raster_feature


@pytest.mark.parametrize("form", [False, True])
@pytest.mark.parametrize("responsive", [False, True])
@pytest.mark.parametrize("raster_fallback", [False, True])
@pytest.mark.parametrize("fit,position,checks", [
    ("contain", "center", [(40, 90, "red"), (40, 70, "white"), (200, 150, "blue")]),
    ("cover", "left center", [(40, 20, "red"), (200, 220, "red")]),
    ("cover", "right center", [(40, 20, "blue"), (200, 220, "blue")]),
    ("fill", "center", [(40, 90, "red"), (40, 70, "white"), (200, 150, "blue")]),
    ("none", "center", [(10, 30, "red"), (230, 210, "blue"), (120, 10, "white")]),
    ("scale-down", "center", [(40, 90, "red"), (40, 70, "white"), (200, 150, "blue")]),
])
def test_replaced_svg_object_fit_viewport(form, responsive, raster_fallback, fit, position, checks):
    _require_pdf_engine()
    if raster_fallback:
        _require_svg_raster_feature()
    svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="600" height="200" viewBox="0 0 600 200">'
           '<defs><filter id="unused"><feGaussianBlur stdDeviation="1"/></filter></defs>'
           '<rect width="200" height="200" fill="#ff0000"/>'
           '<rect x="200" width="200" height="200" fill="#00ff00"/>'
           '<rect x="400" width="200" height="200" fill="#0000ff"/></svg>')
    source = 'data:image/svg+xml;base64,' + base64.b64encode(svg.encode()).decode()
    html = f'<html><body><img src="{source}" alt="Three colored stripes"></body></html>'
    width = "100%" if responsive else "180pt"
    css = ('@page{size:180pt 180pt;margin:0}body{margin:0;background:white}'
           f'img{{display:block;width:{width};height:180pt;object-fit:{fit};object-position:{position}}}')
    engine = fullbleed.PdfEngine(svg_form_xobjects=form, svg_raster_fallback=raster_fallback)
    pages = list(engine.render_image_pages(html, css, 96))
    assert len(pages) == 1
    width, height, rows = _decode_png_rgba(pages[0])
    assert (width, height) == (240, 240)
    colors = {"red": (255, 0, 0), "blue": (0, 0, 255), "white": (255, 255, 255)}
    for x, y, color in checks:
        assert tuple(rows[y][4*x:4*x+3]) == colors[color], (fit, position, responsive, x, y)
    assert pages == list(engine.render_image_pages(html, css, 96))
    assert bytes(engine.render_pdf(html, css)) == bytes(engine.render_pdf(html, css))
