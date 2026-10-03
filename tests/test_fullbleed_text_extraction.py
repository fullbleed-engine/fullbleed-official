"""Regression checks on PDFs emitted by the installed native extension."""
from importlib import resources
import io
from pathlib import Path
import runpy

import fullbleed
import pytest


def test_styled_pdf_text_is_extracted_once(tmp_path):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    pytest.importorskip("pypdf")
    script = Path(__file__).resolve().parents[1] / "tools" / "smoke_text_extraction.py"
    report = runpy.run_path(str(script))["check"](tmp_path)
    assert report["ok"] and len(report["cases"]) == 36


@pytest.mark.parametrize("mode", ["fixed", "reflow", "compact"])
def test_compiled_bold_text_and_previews_survive_repeated_rendering(tmp_path, mode):
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available")
    PdfReader = pytest.importorskip("pypdf").PdfReader
    engine = fullbleed.PdfEngine()
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")),
                    fullbleed.AssetKind.Font, name="Inter")
    engine.register_bundle(bundle)
    compiled = engine.compile_pdf("<h1>Invoice</h1><p>Customer {{name}}</p>",
                                  "@page {size:A4;margin:20mm} body {font-family:Inter}")
    previews = []
    for attempt in range(2):
        bindings = {"name": ["Alpha", "Bravo"]}
        if mode == "fixed":
            pdf = compiled.render_pdf_bindings(bindings)
        else:
            pdf = compiled.render_pdf_reflow_bindings(
                bindings, compression="compact" if mode == "compact" else "throughput")
        texts = [" ".join(page.extract_text().split()) for page in PdfReader(io.BytesIO(pdf)).pages]
        assert texts == ["Invoice Customer Alpha", "Invoice Customer Bravo"]
        path = tmp_path / f"{attempt}.pdf"
        path.write_bytes(pdf)
        previews.append([bytes(png) for png in engine.render_finalized_pdf_image_pages(str(path), 96)])
    assert len(previews[0]) == 2
    assert previews[0] == previews[1], "Cached rendering lost or changed a glyph paint resource"
