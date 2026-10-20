# SPDX-License-Identifier: MIT
"""Verify installed component preparation reuses one owned native layout."""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import closing
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path
import sys

import fullbleed
from fullbleed.ui.experimental import Renderer
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "examples/component_recipes"))
from documents import Invoice, OperationsReport


def trace_content(trace):
    return {key: value for key, value in trace.items() if key != "generated_at_unix_ms"}


def layout_count(path):
    events = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    return Counter(event["name"] for event in events if event["type"] == "perf.span")["layout"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    options = dict(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                   document_timestamp="2026-10-15T00:00:00Z")
    data = json.loads((ROOT / "examples/components/data.json").read_text(encoding="utf-8"))
    report = {**data, "items": [{**data["items"][index % 3], "name": f"Deliverable {index:03d}"} for index in range(1, 61)]}
    cases = []
    for name, document, pages in (("invoice", Invoice(data), 1), ("report", OperationsReport(report), 5)):
        folder = args.out / name
        folder.mkdir(exist_ok=True)
        plan = document.compile(target="pdf")
        engine = Renderer(**options).engine_for(document)
        expected_pdf, glyphs = engine.render_pdf_with_glyph_report(plan.html, plan.css)
        assert not glyphs
        expected_images = engine.render_image_pages(plan.html, plan.css, dpi=72)
        expected_trace = trace_content(engine.export_render_time_pagination_trace(plan.html, plan.css))
        del engine

        log = folder / "prepared.jsonl"
        prepared = document.prepare(Renderer(**options, perf=True, perf_out=str(log)))
        assert isinstance(prepared, fullbleed.CompiledDocument)
        assert prepared.stats()["page_count"] == pages
        pdf = prepared.render_pdf()
        images = prepared.render_image_pages(dpi=72)
        trace = trace_content(prepared.export_render_time_pagination_trace())
        assert pdf == expected_pdf == prepared.render_pdf()
        assert images == expected_images
        assert trace == expected_trace
        del prepared  # flush the owned performance log after all output operations
        assert layout_count(log) == 1

        for operation in ("pdf", "images"):
            direct_log = folder / f"direct-{operation}.jsonl"
            renderer = Renderer(**options, perf=True, perf_out=str(direct_log))
            output = document.render_pdf(renderer) if operation == "pdf" else document.render_image_pages(renderer, dpi=72)
            assert output == (pdf if operation == "pdf" else images)
            assert layout_count(direct_log) == 1

        (folder / "document.pdf").write_bytes(pdf)
        (folder / "pagination.json").write_text(json.dumps(trace, indent=2) + "\n", encoding="utf-8")
        document.emit_artifacts(html_path=folder / "document.html", css_path=folder / "document.css", target="pdf")
        for index, png in enumerate(images, 1):
            (folder / f"page-{index}.png").write_bytes(png)
        reader = PdfReader(BytesIO(pdf))
        assert len(reader.pages) == pages
        assert reader.metadata.title == document.title
        assert reader.trailer["/Root"]["/Lang"] == document.lang
        with closing(pdfium.PdfDocument(pdf)) as independent:
            for index in range(len(independent)):
                with closing(independent[index]) as page, closing(page.render(scale=1)) as bitmap:
                    bitmap.to_pil().save(folder / f"independent-page-{index + 1}.png")
        cases.append(dict(name=name, pages=pages, pdf_sha256=sha256(pdf).hexdigest(),
                          image_sha256=[sha256(png).hexdigest() for png in images],
                          layout_counts={"prepared_outputs": 1, "direct_pdf": 1, "direct_images": 1},
                          trace_matches=True))
    result = {"ok": True, "installed_module": fullbleed.__file__, "cases": cases}
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
