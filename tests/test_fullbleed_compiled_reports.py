"""Compiled reports describe final record text and leave retained PDFs unchanged."""
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


CSS = "@page { size:400pt 200pt; margin:12pt; } body { font:12pt Latin; }"


def engine(fallback=False):
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(files("fullbleed_assets").joinpath("fonts", "NotoSans-Regular.ttf")),
                    fullbleed.AssetKind.Font, name="Latin", trusted=True)
    if fallback:
        bundle.add_file(str(Path(__file__).parent / "fixtures/table-wrapping/FullbleedCJKTest-Regular.ttf"),
                        fullbleed.AssetKind.Font, name="CJK", trusted=True)
    renderer = fullbleed.PdfEngine()
    renderer.register_bundle(bundle)
    return renderer


def methods(template, mode):
    name = "render_pdf_bindings" if mode == "fixed" else "render_pdf_reflow_bindings"
    return getattr(template, name), getattr(template, name + "_with_report")


def inspect(data, report, directory, name):
    (directory / f"{name}.pdf").write_bytes(data)
    (directory / f"{name}-report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    reader = PdfReader(BytesIO(data), strict=True)
    assert report["page_count"] == len(reader.pages)
    offset = 0
    for index, row in enumerate(report["records"]):
        assert row["record_index"] == index
        assert row["page_offset"] == offset
        assert row["page_count"] > 0
        offset += row["page_count"]
    assert offset == len(reader.pages)
    document = pdfium.PdfDocument(data)
    pages = []
    try:
        assert len(document) == len(reader.pages)
        for index, parsed in enumerate(reader.pages):
            page = document[index]
            bitmap = page.render(scale=1)
            image = bitmap.to_pil().convert("RGB")
            image.save(directory / f"{name}-{index+1}.png")
            pages.append(dict(text=parsed.extract_text(), size=list(image.size),
                              pixels_sha256=sha256(image.tobytes()).hexdigest()))
            image.close()
            bitmap.close()
            page.close()
    finally:
        document.close()
    (directory / f"{name}-readers.json").write_text(json.dumps(pages, indent=2), encoding="utf-8")
    return pages


@pytest.mark.parametrize("mode", ["fixed", "reflow"])
@pytest.mark.parametrize("fallback", [False, True])
def test_compiled_report_bound_font_coverage_rebinding_and_replay(tmp_path, mode, fallback):
    template = engine(fallback).compile_pdf("<p>Marker {{name}} end</p>", CSS)
    ordinary, diagnosed = methods(template, mode)
    original = None
    for index, values in enumerate((["ASCII", "東京", "Café"], ["東京", "BRAVO", "東京"], ["ASCII", "東京", "Café"])):
        bindings = {"name": values}
        expected = ordinary(bindings)
        pdf, report = diagnosed(bindings)
        assert isinstance(pdf, bytes) and pdf == expected
        assert len(report["records"]) == len(values)
        for value, row in zip(values, report["records"]):
            expected_missing = {ord("東"), ord("京")} if value == "東京" and (mode == "fixed" or not fallback) else set()
            assert {glyph["codepoint"] for glyph in row["missing_glyphs"]} == expected_missing
            assert all(glyph["count"] >= 1 and glyph["fonts_tried"] for glyph in row["missing_glyphs"])
        pages = inspect(pdf, report, tmp_path, f"batch-{index}")
        assert all("Marker" in page["text"] and "end" in page["text"] for page in pages)
        if index == 0:
            original = (pdf, report)
        elif index == 2:
            assert (pdf, report) == original


@pytest.mark.parametrize("mode", ["fixed", "reflow"])
def test_compiled_report_checks_holes_in_registered_font_metrics(tmp_path, mode):
    template = engine(True).compile_pdf("<p>{{name}}</p>", CSS.replace("12pt Latin", "12pt CJK, Latin"))
    ordinary, diagnosed = methods(template, mode)
    bindings = {"name": ["ABC", "Café"]}
    pdf, report = diagnosed(bindings)
    assert pdf == ordinary(bindings)
    assert report["records"][0]["missing_glyphs"] == []
    expected = {ord("é")} if mode == "fixed" else set()
    assert {glyph["codepoint"] for glyph in report["records"][1]["missing_glyphs"]} == expected
    inspect(pdf, report, tmp_path, mode)


@pytest.mark.parametrize("effect", ["opacity:.5", "filter:opacity(.5)", "transform:rotate(2deg)"])
def test_compiled_report_includes_nested_and_generated_text(tmp_path, effect):
    template = engine().compile_pdf(
        f"<div style='{effect}'><p>水</p></div><p class='bound'>{{{{name}}}}</p>",
        CSS + " .bound::before { content:'京'; }",
    )
    bindings = {"name": ["ALPHA", "東京"]}
    pdf, report = template.render_pdf_reflow_bindings_with_report(bindings)
    assert pdf == template.render_pdf_reflow_bindings(bindings)
    assert [{glyph["char"] for glyph in row["missing_glyphs"]} for row in report["records"]] == [{"京", "水"}, {"京", "東", "水"}]
    inspect(pdf, report, tmp_path, "nested")


@pytest.mark.parametrize("compression", [fullbleed.CompiledFlowCompression.Throughput, fullbleed.CompiledFlowCompression.Compact])
def test_compiled_report_returns_variable_page_ranges(tmp_path, compression):
    template = engine().compile_pdf("<p>{{name}}</p>", CSS)
    bindings = {"name": ["SHORT", "A long document record. " * 160, "END"]}
    pdf, report = template.render_pdf_reflow_bindings_with_report(bindings, compression=compression)
    assert pdf == template.render_pdf_reflow_bindings(bindings, compression=compression)
    pages = inspect(pdf, report, tmp_path, "variable-pages")
    assert [row["page_count"] == 1 for row in report["records"]] == [True, False, True]
    assert "SHORT" in pages[0]["text"] and "END" in pages[-1]["text"]


@pytest.mark.parametrize("mode", ["fixed", "reflow"])
def test_compiled_report_invalid_columns_and_hash_recovery(tmp_path, mode):
    template = engine().compile_pdf("<p>{{name}} {{code}}</p>", CSS)
    ordinary, diagnosed = methods(template, mode)
    hash_path = tmp_path / "output.sha256"
    for invalid in ({}, {"name": ["A"]}, {"name": ["A"], "code": []}, {"name": [], "code": []}):
        with pytest.raises((ValueError, RuntimeError)):
            diagnosed(invalid, deterministic_hash=str(hash_path))
        assert not hash_path.exists()
    bindings = {"name": ["ALPHA"], "code": ["東京"]}
    pdf, report = diagnosed(bindings, deterministic_hash=str(hash_path))
    assert pdf == ordinary(bindings)
    assert hash_path.read_text().strip() == sha256(pdf).hexdigest()
    assert {glyph["char"] for glyph in report["records"][0]["missing_glyphs"]} == {"京", "東"}
    inspect(pdf, report, tmp_path, "recovered")


@pytest.mark.parametrize("mode", ["fixed", "reflow"])
def test_compiled_report_documents_base14_coverage_limit(tmp_path, mode):
    template = fullbleed.PdfEngine().compile_pdf("<p>{{name}}</p>", CSS.replace("Latin", "Helvetica"))
    ordinary, diagnosed = methods(template, mode)
    bindings = {"name": ["ASCII", "東京"]}
    pdf, report = diagnosed(bindings)
    assert pdf == ordinary(bindings)
    assert all(not row["missing_glyphs"] for row in report["records"])
    inspect(pdf, report, tmp_path, "base14")
