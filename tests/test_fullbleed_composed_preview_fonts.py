"""Separate PDF font subsets must retain their glyphs after composition."""
from importlib.resources import files
from io import BytesIO

import fullbleed
from pypdf import PdfReader, PdfWriter
import pytest


@pytest.mark.parametrize("font", ["Inter-Variable.ttf", "NotoSans-Regular.ttf"])
@pytest.mark.parametrize("weight", [400, 700])
@pytest.mark.parametrize("reverse", [False, True])
def test_composed_preview_preserves_font_subsets(tmp_path, font, weight, reverse):
    path = files("fullbleed_assets").joinpath("fonts", font)
    family = "Inter" if font.startswith("Inter") else "Noto Sans"
    engine = fullbleed.PdfEngine(font_files=[str(path)])
    css = f"""@page {{ size: 360pt 150pt; margin: 0; }}
body {{ margin: 0; font-family: '{family}'; font-size: 22pt; font-weight: {weight}; }}
p {{ position: absolute; left: 12pt; margin: 0; }}
.top {{ top: 12pt; }} .bottom {{ top: 82pt; }}
"""
    labels = ["ABCD Zoë", "WXYZ 0123456789 -"]
    if reverse:
        labels.reverse()
    template_html = f"<p class='top'>{labels[0]}</p>"
    overlay_html = f"<p class='bottom'>{labels[1]}</p>"
    template = bytes(engine.render_pdf(template_html, css))
    overlay = bytes(engine.render_pdf(overlay_html, css))
    control = bytes(engine.render_pdf(template_html + overlay_html, css))
    template_path = tmp_path / "template.pdf"
    overlay_path = tmp_path / "overlay.pdf"
    control_path = tmp_path / "control.pdf"
    output_path = tmp_path / "composed.pdf"
    template_path.write_bytes(template)
    overlay_path.write_bytes(overlay)
    control_path.write_bytes(control)
    result = fullbleed.finalize_compose_pdf(
        [("background", str(template_path))],
        [("background", 0, 0, 0.0, 0.0)],
        str(overlay_path), str(output_path), annotation_mode="none",
    )
    assert result["pages_written"] == 1
    for data in [output_path.read_bytes(), control]:
        reader = PdfReader(BytesIO(data), strict=True)
        assert len(reader.pages) == 1
        text = reader.pages[0].extract_text()
        assert all(label in text for label in labels), text
    actual = [bytes(png) for png in engine.render_finalized_pdf_image_pages(str(output_path), 96)]
    expected = [bytes(png) for png in engine.render_finalized_pdf_image_pages(str(control_path), 96)]
    assert len(actual) == len(expected) == 1
    (tmp_path / "actual.png").write_bytes(actual[0])
    (tmp_path / "expected.png").write_bytes(expected[0])
    assert actual == expected, "The final composed preview must preserve both font subsets."
    assert template_path.read_bytes() == template


@pytest.mark.parametrize("font", ["Inter-Variable.ttf", "NotoSans-Regular.ttf"])
def test_merged_pages_preserve_independent_font_subsets(tmp_path, font):
    path = files("fullbleed_assets").joinpath("fonts", font)
    family = "Inter" if font.startswith("Inter") else "Noto Sans"
    engine = fullbleed.PdfEngine(font_files=[str(path)])
    css = f"""@page {{ size: 360pt 150pt; margin: 12pt; }}
body {{ margin: 0; font-family: '{family}'; font-size: 22pt; }}
p {{ margin: 0; }}
"""
    labels = ["ABCD Zoë", "WXYZ 0123456789 -"]
    writer = PdfWriter()
    expected = []
    for index, label in enumerate(labels):
        source = tmp_path / f"source-{index}.pdf"
        source.write_bytes(bytes(engine.render_pdf(f"<p>{label}</p>", css)))
        writer.append(str(source))
        expected.extend(bytes(png) for png in engine.render_finalized_pdf_image_pages(str(source), 96))
    merged = tmp_path / "merged.pdf"
    writer.write(merged)
    writer.close()
    reader = PdfReader(merged, strict=True)
    assert len(reader.pages) == 2
    for page, label in zip(reader.pages, labels):
        assert label in page.extract_text()
    actual = [bytes(png) for png in engine.render_finalized_pdf_image_pages(str(merged), 96)]
    assert len(actual) == len(expected) == 2
    for index, (observed, reference) in enumerate(zip(actual, expected)):
        (tmp_path / f"actual-{index}.png").write_bytes(observed)
        (tmp_path / f"expected-{index}.png").write_bytes(reference)
    assert actual == expected, "Merged pages must paint the same glyphs as their source PDFs."
