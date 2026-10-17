# SPDX-License-Identifier: MIT
"""Verify component styling, named parts and HTML exports at the SVG boundary."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
from io import BytesIO
import json
from pathlib import Path

from fullbleed.ui.experimental import Box, Document, Node, Page, Paragraph, Renderer, component, pt, style, styled
from PIL import Image
from pypdf import PdfReader
import pypdfium2 as pdfium


@component(parts=("paint",))
def BrandMark():
    shape = Node("rect", attributes=(("class", "mark"), ("width", "100"), ("height", "100"), ("fill", "#0060c0")))
    group = Node("g", (shape, shape), part="paint")
    picture = Node("svg", (group,), attributes=(("width", "100"), ("height", "100"), ("role", "img"), ("aria-label", "Brand mark")))
    return Box(picture, Paragraph("Brand sample"), class_name="brand")


def check(out: Path):
    out.mkdir(parents=True, exist_ok=True)
    Mark = styled(BrandMark, color="#d00000")
    original = Mark()
    faded = original.with_parts(paint=style(opacity=0.5))
    page = Page((pt(300), pt(200)), margin=pt(0))
    rules = "svg {display:block} .brand .mark {fill:currentColor}"
    documents = {
        "class-inherited-color": (Document(original, page=page, styles=rules, title="Brand sample"), [208, 0, 0]),
        "part-group-opacity": (Document(faded, page=page, styles=rules, title="Brand sample"), [231, 128, 128]),
    }
    original_source = documents["class-inherited-color"][0].compile()
    records = []
    for name, (document, expected) in documents.items():
        for forms in (False, True):
            variant = "form" if forms else "vector"
            folder = out / name / variant
            folder.mkdir(parents=True, exist_ok=True)
            renderer = Renderer(svg_form_xobjects=forms, svg_raster_fallback=False,
                                document_timestamp="2026-10-16T00:00:00Z")
            prepared = document.prepare(renderer)
            pdf = prepared.render_pdf()
            assert pdf == document.render_pdf(renderer)
            styled_html = document.to_html(include_styles=True)
            plain_html = document.to_html(include_styles=False)
            assert "<style>" in styled_html and "<style>" not in plain_html and " style=" not in plain_html
            assert 'class="mark"' in plain_html and 'fill="#0060c0"' in plain_html
            (folder / "styled.html").write_text(styled_html, encoding="utf-8")
            (folder / "unstyled.html").write_text(plain_html, encoding="utf-8")
            exported = bytes(renderer.engine_for(document).render_pdf(styled_html, ""))
            assert pdf == exported, (name, variant, "styled export PDF")
            previews = prepared.render_image_pages(dpi=96)
            assert len(previews) == 1
            with Image.open(BytesIO(previews[0])) as image:
                native = list(image.convert("RGB").getpixel((24, 24)))
            (folder / "native.png").write_bytes(previews[0])
            host_css = "@page {size:300pt 200pt;margin:0} body {margin:0} svg {display:block} .brand .mark {fill:#205b47}"
            hosted = bytes(renderer.engine_for(document).render_pdf(plain_html, host_css))
            for mode, data, color in (("document", pdf, expected), ("styled-export", exported, expected), ("host-styled-export", hosted, [32, 91, 71])):
                (folder / (mode + ".pdf")).write_bytes(data)
                reader = PdfReader(BytesIO(data))
                assert len(reader.pages) == 1
                assert "Brand sample" in reader.pages[0].extract_text()
                with closing(pdfium.PdfDocument(data)) as parsed:
                    with closing(parsed[0]) as image_page, closing(image_page.render(scale=96 / 72)) as bitmap:
                        picture = bitmap.to_pil()
                        observed = list(picture.convert("RGB").getpixel((24, 24)))
                        picture.save(folder / (mode + ".png"))
                assert max(abs(a - b) for a, b in zip(observed, color)) <= 2, (name, variant, mode, observed, color)
                records.append({"case": name, "variant": variant, "mode": mode, "rgb": observed,
                                "expected": color, "pdf_sha256": sha256(data).hexdigest(), "ok": True})
            assert max(abs(a - b) for a, b in zip(native, expected)) <= 2
    assert documents["class-inherited-color"][0].compile() == original_source
    report = {"ok": True, "version": version("fullbleed"), "checks": len(records), "records": records,
              "component_api": "Python nodes, reusable defaults, class selectors and named-part overrides; no HTML strings",
              "exports": "Styled HTML reproduces document PDF bytes; unstyled HTML accepts a host's CSS without mutating the source."}
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    result = check(parser.parse_args().out)
    print(json.dumps({key: value for key, value in result.items() if key != "records"}))
