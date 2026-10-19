# SPDX-License-Identifier: MIT
"""Check source/layout identity and authored semantics in an installed wheel."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path
import warnings
from contextlib import closing

import fullbleed
from fullbleed.ui.accessibility import A11yValidationError, A11yWarning
from fullbleed.ui.experimental import (
    AuthoringError, Column, DataTable, Document, DocumentSource, Flow, Heading,
    Page, Paragraph, Renderer, Strong, Theme, component, mm, pt, style,
)
from pypdf import PdfReader
import pypdfium2 as pdfium


@component
def Summary(*children):
    return Flow(Heading("Project summary", level=1), *children, gap=pt(8))


@component
def DuplicateIds():
    return Flow(Paragraph("First", attrs={"id": "same"}, part="first"),
                Paragraph("Second", attrs={"id": "same"}, part="second"))


@component
def BrokenParagraph():
    return Paragraph("Before", Paragraph("Nested block"), "After")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
                        document_timestamp="2026-10-16T00:00:00Z")
    document = Document(Summary(
        Paragraph("Prepared for ", Strong("Maple & Finch")),
        DataTable([("Design", 3), ("Review", 1)], caption="Completed work", columns=[
            Column("Work", lambda item: item[0]), Column("Hours", lambda item: item[1], align="right"),
        ]), Paragraph("Thank you.")),
        title="Component semantics", page=Page("A5", margin=mm(15)),
        theme=Theme(text=style(font_family="Inter", font_size=pt(10))),
    )
    source = document.compile(a11y_mode="raise")
    assert isinstance(source, DocumentSource) and not hasattr(source, "render_pdf")
    assert source.a11y_report["ok"]
    prepared = document.prepare(renderer, a11y_mode="raise")
    assert isinstance(prepared, fullbleed.CompiledDocument)
    pdf = prepared.render_pdf()
    assert pdf == document.render_pdf(renderer, a11y_mode="raise")
    assert pdf == document.render_pdf(renderer)
    images = prepared.render_image_pages(dpi=96)
    assert images == document.render_image_pages(renderer, dpi=96, a11y_mode="raise")
    reader = PdfReader(BytesIO(pdf))
    assert len(reader.pages) == 1
    text = reader.pages[0].extract_text()
    assert all(value in text for value in ("Project summary", "Maple & Finch", "Work", "Hours", "Design", "Review"))
    assert abs(float(reader.pages[0].mediabox.width) - 419.528) < .1
    (args.out / "summary.pdf").write_bytes(pdf)
    (args.out / "summary.png").write_bytes(images[0])
    with closing(pdfium.PdfDocument(pdf)) as independent:
        with closing(independent[0]) as page:
            with closing(page.render(scale=96 / 72)) as bitmap:
                bitmap.to_pil().save(args.out / "summary-pdf.png")

    invalid = Document(DuplicateIds())
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", A11yWarning)
        warned = invalid.compile(a11y_mode="warn")
    assert len(caught) == 1 and issubclass(caught[0].category, A11yWarning)
    diagnostic = warned.a11y_report["diagnostics"][0]
    assert diagnostic["code"] == "ID_DUPLICATE"
    assert diagnostic["source"]["path"] == ["DuplicateIds", "second"]
    assert diagnostic["related_sources"][0]["path"] == ["DuplicateIds", "first"]
    copy = warned.a11y_report
    copy["diagnostics"].clear()
    assert warned.a11y_report["diagnostics"]
    errors = []
    for name in ("prepare", "render_pdf", "render_image_pages"):
        try:
            getattr(invalid, name)(renderer, a11y_mode="raise")
        except A11yValidationError as error:
            assert error.report["diagnostics"][0] == diagnostic
            errors.append(name)
        else:
            raise AssertionError(f"{name} accepted duplicate IDs in strict mode")
    for name in ("compile", "render_pdf", "render_image_pages"):
        try:
            method = getattr(Document(BrokenParagraph()), name)
            method() if name == "compile" else method(renderer)
        except AuthoringError as error:
            assert error.code == "structure.child"
            assert error.path == ("BrokenParagraph", "p")
            errors.append("structure." + name)
        else:
            raise AssertionError(f"{name} accepted nested paragraph blocks")
    result = {
        "ok": True, "installed_module": fullbleed.__file__,
        "source_type": type(source).__name__, "prepared_type": type(prepared).__name__,
        "valid_pdf_matches_unchecked_output": True, "pdf_sha256": sha256(pdf).hexdigest(),
        "page_box_pt": [float(reader.pages[0].mediabox.width), float(reader.pages[0].mediabox.height)],
        "text": text, "strict_rejections": errors, "semantic_diagnostic": diagnostic,
    }
    (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in result.items() if key != "semantic_diagnostic"}, indent=2))


if __name__ == "__main__":
    main()
