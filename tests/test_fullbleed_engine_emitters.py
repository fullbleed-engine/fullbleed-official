from __future__ import annotations

from pathlib import Path

import pytest

import fullbleed


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def test_pdf_engine_emit_artifacts_preserves_document_metadata(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        document_lang="en-US",
        document_title='Engine "Doc" <A&B>',
    )
    assert engine.document_lang == "en-US"
    assert engine.document_title == 'Engine "Doc" <A&B>'

    engine.document_lang = "fr-CA"
    engine.document_title = 'Updated "Title" <x&y>'
    meta = engine.document_metadata()

    assert meta["document_lang"] == "fr-CA"
    assert meta["document_title"] == 'Updated "Title" <x&y>'

    html_path = tmp_path / "out" / "doc.html"
    css_path = tmp_path / "out" / "doc.css"
    body_html = '<main data-fb-role="document-root"><p>payload</p></main>'
    css = "@page { size: letter; }\nbody { color: #111; }"

    result = engine.emit_artifacts(
        body_html,
        css,
        str(html_path),
        str(css_path),
    )

    html_text = html_path.read_text(encoding="utf-8")
    css_text = css_path.read_text(encoding="utf-8")

    assert result["html_path"] == str(html_path)
    assert result["css_path"] == str(css_path)
    assert result["html"] == html_text
    assert result["css"] == css_text
    assert css_text == css
    assert '<html lang="fr-CA">' in html_text
    assert "<title>Updated &quot;Title&quot; &lt;x&amp;y&gt;</title>" in html_text
    assert body_html in html_text


def test_pdf_engine_document_metadata_properties_accept_none(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Alpha")
    engine.document_lang = None
    engine.document_title = None

    assert engine.document_lang is None
    assert engine.document_title is None

    html_path = tmp_path / "raw.html"
    css_path = tmp_path / "raw.css"
    engine.emit_artifacts("<div>x</div>", "body{}", str(html_path), str(css_path))
    html_text = html_path.read_text(encoding="utf-8")
    assert '<html lang="en">' in html_text
    assert "<title>fullbleed document</title>" in html_text

