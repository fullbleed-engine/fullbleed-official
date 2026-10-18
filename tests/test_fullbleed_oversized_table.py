"""Inspect actual glyph placement and logical cells across table fragments."""
from pathlib import Path
import subprocess
import sys
from importlib import resources
from hashlib import sha256

import pytest


def test_oversized_table_cells_preserve_visible_content_and_header_links(tmp_path):
    pytest.importorskip("pypdf")
    pytest.importorskip("pypdfium2")
    script = Path(__file__).resolve().parents[1]/"tools/smoke_oversized_table.py"
    result = subprocess.run([
        sys.executable, "-I", str(script), "--out", str(tmp_path/"oversized-table"),
        "--case", "original-baseline", "--case", "mixed-columns",
        "--case", "fragmented-row-header", "--case", "repeated-footer", "--case", "list-cell",
        "--profile", "tagged", "--mode", "direct", "--mode", "reflow", "--mode", "compact",
    ], capture_output=True, text=True, timeout=120)
    assert result.returncode == 0, result.stdout + result.stderr


@pytest.mark.parametrize("compression", ["throughput", "compact"])
@pytest.mark.parametrize("weight", [400, 700])
def test_reflow_cached_pages_keep_fonts_across_replay_and_different_batches(tmp_path, compression, weight):
    pdfium = pytest.importorskip("pypdfium2")
    import fullbleed

    fonts = resources.files("fullbleed_assets").joinpath("fonts")
    html = "<h1>{{name}}</h1><div><p>STATIC 0123456789 Garden together.</p></div>"
    css = """@page {size:320pt 160pt;margin:18pt}
        body {font-family:'Noto Sans';font-size:10pt}
        h1 {font-family:Inter;font-size:16pt} div {break-before:page}"""
    css += f" h1 {{font-weight:{weight}}}"

    def engine():
        return fullbleed.PdfEngine(font_files=[str(fonts.joinpath("Inter-Variable.ttf")),
            str(fonts.joinpath("NotoSans-Regular.ttf"))])

    def compile_document():
        return engine().compile_pdf(html, css)

    compiled = compile_document()
    first = {"name": ["Ada", "Bea"]}
    changed = {"name": ["Zoe", "Quinn"]}
    def render(document, bindings):
        return bytes(document.render_pdf_reflow_bindings(bindings, compression=compression))

    results = {
        "first": render(compiled, first),
        "replay": render(compiled, first),
        "changed": render(compiled, changed),
        "changed-cold": render(compile_document(), changed),
        "first-again": render(compiled, first),
    }
    for name, data in results.items():
        (tmp_path/(name+".pdf")).write_bytes(data)
    def inspect(data):
        with pdfium.PdfDocument(data) as pdf:
            texts = []
            images = []
            for page in pdf:
                text = page.get_textpage()
                texts.append(text.get_text_range())
                bitmap = page.render(scale=1)
                images.append(sha256(bytes(bitmap.buffer)).hexdigest())
                bitmap.close(); text.close(); page.close()
        return texts, images

    expected = {}
    for value in [*first["name"], *changed["name"]]:
        data = bytes(engine().render_pdf(html.replace("{{name}}", value), css))
        (tmp_path/(value+"-single.pdf")).write_bytes(data)
        expected[value] = inspect(data)[1]

    pixels = {}
    for name, data in results.items():
        texts, images = inspect(data)
        assert "\n".join(texts).count("STATIC 0123456789 Garden together.") == 2, name
        values = changed["name"] if name.startswith("changed") else first["name"]
        assert [text.strip() for text in texts[::2]] == values, name
        assert images == [page for value in values for page in expected[value]], name
        pixels[name] = images
    assert results["first"] == results["replay"] == results["first-again"]
    assert results["changed"] == results["changed-cold"]
    assert pixels["first"] == pixels["replay"] == pixels["first-again"]
    assert pixels["changed"] == pixels["changed-cold"]


@pytest.mark.parametrize("profile", ["none", "pdfua1"])
@pytest.mark.parametrize("presentation", [False, True])
def test_reflow_table_columns_match_individual_record_layouts(tmp_path, profile, presentation):
    pdfium = pytest.importorskip("pypdfium2")
    import fullbleed

    font = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(font_files=[str(font)], pdf_profile=profile,
        document_title="Table records", document_lang="en-US")
    html = "<table><thead><tr><th>Name</th><th>Status</th></tr></thead><tbody><tr><td>{{name}}</td><td>READY</td></tr></tbody></table>"
    if presentation:
        html = html.replace("<table>", '<table role="presentation">')
    css = """@page {size:320pt 160pt;margin:18pt}
        body {font-family:Inter;font-size:10pt} table {width:100%;border-collapse:collapse}
        th,td {padding:5pt;text-align:left}"""
    values = ["Ada", "Alexandria Chen", "Bea"]

    def pixels(label, data):
        (tmp_path/(label+".pdf")).write_bytes(data)
        images = []
        with pdfium.PdfDocument(data) as pdf:
            for page in pdf:
                bitmap = page.render(scale=2)
                images.append(sha256(bytes(bitmap.buffer)).hexdigest())
                bitmap.close(); page.close()
        return images

    expected = [page for index, value in enumerate(values) for page in pixels(
        f"single-{index}", bytes(engine.render_pdf(html.replace("{{name}}", value), css)))]
    compiled = engine.compile_pdf(html, css)
    batch = bytes(compiled.render_pdf_reflow_bindings({"name": values}))
    replay = bytes(compiled.render_pdf_reflow_bindings({"name": values}))
    assert pixels("batch", batch) == expected
    assert pixels("replay", replay) == expected
    assert batch == replay
