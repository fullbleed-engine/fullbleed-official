# SPDX-License-Identifier: MIT
"""Verify that component font choices survive CSS variables and HTML export.

PDF readers are development dependencies, not package dependencies.
"""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from importlib.metadata import version
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

from fullbleed.ui.experimental import Document, Page, Paragraph, Renderer, Theme, pt, style, styled, token
from fullbleed.ui.experimental import bootstrap as bs
from pypdf import PdfReader
import pypdfium2 as pdfium

TEXT = "Ready for print 1042"
HOST_CSS = "body {--legacy:Courier;font-family:var(--legacy)} body {font-family:Inter}"


def authored():
    theme = Theme(text=style(font_family="Inter", font_size=pt(12)))
    shared = dict(page=Page((pt(300), pt(200)), margin=pt(18)), title="Component font choices")
    return {
        "plain-theme": Document(Paragraph(TEXT), theme=theme, **shared),
        "bootstrap-theme": Document(bs.Card(TEXT), theme=theme, **shared),
        "bootstrap-token-theme": Document(bs.Card(TEXT), theme=Theme({"brand_font": "Inter"}, text=style(font_family=token("brand_font"))), **shared),
        "root-override": Document(bs.Card(TEXT, class_name="invoice").with_style(font_family="Inter"),
            styles=".invoice {--legacy:Courier;font-family:var(--legacy)}", **shared),
        "body-part-override": Document(bs.Card(TEXT).with_parts(body=style(font_family="Inter")),
            styles=".card-body {--legacy:Courier;font-family:var(--legacy)}", **shared),
        "literal-class": Document(bs.Card(TEXT, class_name="invoice"),
            styles=".invoice {--legacy:Courier;font-family:var(--legacy)} .invoice {font-family:Inter}", **shared),
        "reusable-default": Document(styled(bs.Card, font_family="Inter")(TEXT), **shared),
        "font-shorthand": Document(bs.Card(TEXT), styles="body {font:12px Inter}", **shared),
    }


def embedded_fonts(data):
    reader = PdfReader(BytesIO(data))
    assert len(reader.pages) == 1
    assert TEXT in reader.pages[0].extract_text()
    result = []
    for reference in reader.pages[0]["/Resources"]["/Font"].values():
        font = reference.get_object()
        descendant = font["/DescendantFonts"][0].get_object() if "/DescendantFonts" in font else font
        assert "/FontDescriptor" in descendant, "Chosen font fell back to a viewer-provided font"
        descriptor = descendant["/FontDescriptor"].get_object()
        assert "/FontFile2" in descriptor, "Chosen font must be embedded, not a viewer substitute"
        assert "Inter" in str(descriptor["/FontName"])
        result.append(str(descriptor["/FontName"]))
    assert result
    return result


def check(out):
    out.mkdir(parents=True, exist_ok=True)
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-17T00:00:00Z")
    records = []
    for name, document in authored().items():
        folder = out / name
        folder.mkdir(exist_ok=True)
        source = document.compile(target="pdf")
        engine = renderer.engine_for(document)
        trace = engine.export_render_time_font_resolution_trace(source.html, source.css)
        (folder / "font-trace.json").write_text(json.dumps(trace, indent=2) + "\n", encoding="utf-8")
        assert trace["fonts"] and all(f["requested_name"] == "Inter" and
            f["pdf_target"]["outcome"] == "registered" and f["pdf_target"]["resolved_name"] == "Inter-Regular"
            for f in trace["fonts"]), (name, trace["fonts"])
        prepared = document.prepare(renderer)
        pdf = document.render_pdf(renderer)
        assert pdf == prepared.render_pdf() == document.render_pdf(renderer)
        styled_html = document.to_html(include_styles=True)
        plain_html = document.to_html(include_styles=False)
        assert "<script" not in styled_html.lower() and "<script" not in plain_html.lower()
        assert "<style" not in plain_html and " style=" not in plain_html
        assert bytes(engine.render_pdf(styled_html, "")) == pdf
        host_pdf = bytes(engine.render_pdf(plain_html, HOST_CSS))
        fonts, host_fonts = embedded_fonts(pdf), embedded_fonts(host_pdf)
        (folder / "document.pdf").write_bytes(pdf)
        (folder / "host-styled.pdf").write_bytes(host_pdf)
        (folder / "styled.html").write_text(styled_html, encoding="utf-8", newline="\n")
        (folder / "unstyled.html").write_text(plain_html, encoding="utf-8", newline="\n")
        (folder / "host.css").write_text(HOST_CSS, encoding="utf-8", newline="\n")
        for mode, previews in (("native", prepared.render_image_pages(dpi=96)),
                ("finalized", engine.render_finalized_pdf_image_pages(str(folder / "document.pdf"), 96))):
            assert len(previews) == 1
            (folder / (mode + ".png")).write_bytes(bytes(previews[0]))
        with closing(pdfium.PdfDocument(pdf)) as parsed, closing(parsed[0]) as page, closing(page.render(scale=96 / 72)) as bitmap:
            bitmap.to_pil().save(folder / "pdfium.png")
        assert document.compile(target="pdf") == source
        records.append(dict(name=name, fonts=fonts, host_fonts=host_fonts, pdf_sha256=sha256(pdf).hexdigest(),
            host_pdf_sha256=sha256(host_pdf).hexdigest(), deterministic=True, styled_export_equal=True,
            unstyled_accepts_host_css=True, font_embedded=True))
    result = dict(ok=True, version=version("fullbleed"), cases=records,
        scope="Eight component font choices and styled/unstyled static exports; not complete CSS font conformance.")
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps(dict(ok=result["ok"], version=result["version"], cases=len(result["cases"]))))


if __name__ == "__main__":
    main()
