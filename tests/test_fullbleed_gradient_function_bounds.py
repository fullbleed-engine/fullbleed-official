"""Gradient discontinuities must not create empty interior PDF subdomains."""
import re

import pytest

import fullbleed


BACKGROUNDS = [
    "linear-gradient(90deg,red 0 25%,gold 25% 50%,blue 50% 75%,green 75% 100%)",
    "linear-gradient(90deg,red 0%,gold 40%,blue 40%,green 100%)",
    "linear-gradient(90deg,red 0 50%,gold 50%,blue 50% 100%)",
    "linear-gradient(90deg,red 0 49.99999%,gold 50.00001%,blue 50.00002% 100%)",
    "linear-gradient(90deg,red 0%,gold 0%,blue 100%)",
    "linear-gradient(90deg,red 0%,gold 100%,blue 100%)",
    "linear-gradient(90deg,rgba(255,0,0,.25) 0 50%,rgba(0,0,255,.75) 50% 100%)",
    "linear-gradient(90deg,red,gold,blue)",
]


def assert_stitching_domains(pdf):
    # These writer-generated function dictionaries are standalone objects.
    # Check the serialized PDF, including values rounded by its number formatter.
    dictionaries = re.findall(rb"<< /FunctionType 3 ([^<>]*) >>", pdf)
    for dictionary in dictionaries:
        bounds = [float(value) for value in re.search(rb"/Bounds \[([^]]*)\]", dictionary)[1].split()]
        functions = re.findall(rb"\d+ 0 R", re.search(rb"/Functions \[([^]]*)\]", dictionary)[1])
        encode = re.search(rb"/Encode \[([^]]*)\]", dictionary)[1].split()
        assert len(functions) == len(bounds) + 1
        assert len(encode) == 2 * len(functions)
        # ISO 32000-1 7.10.4 permits the final bound to equal Domain[1].
        # Interior bounds must increase; a hard stop joins adjacent functions.
        assert all(0 < value <= 1 for value in bounds), bounds
        assert all(left < right for left, right in zip(bounds, bounds[1:])), bounds
    return len(dictionaries)


@pytest.mark.parametrize("background", BACKGROUNDS)
def test_gradient_stitching_domains_in_installed_pdf(background):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    engine = fullbleed.PdfEngine()
    html = "<!doctype html><html><body><div></div></body></html>"
    css = "@page{size:144pt 40pt;margin:0}body{margin:0}div{width:144pt;height:40pt;background:" + background + "}"
    direct = bytes(engine.render_pdf(html, css))
    compiled = bytes(engine.compile_pdf(html, css).render_pdf())
    assert direct == compiled
    count = assert_stitching_domains(direct)
    assert b"/FunctionType 2" in direct or count


def test_repeated_border_gradient_has_valid_stitching_domains():
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    engine = fullbleed.PdfEngine()
    html = "<!doctype html><html><body><div></div></body></html>"
    css = """
        @page { size: 240px 120px; margin: 0 }
        body { margin: 0; padding: 24px }
        div { box-sizing: border-box; width: 192px; height: 72px;
              border: 8px solid transparent;
              border-image: linear-gradient(to right,#e63946 0 25%,#ffbe0b 25% 50%,
                  #457b9d 50% 75%,#2a9d8f 75% 100%) 32 / 1 / 0 round stretch; }
    """
    pdf = bytes(engine.render_pdf(html, css))
    assert assert_stitching_domains(pdf) > 0
