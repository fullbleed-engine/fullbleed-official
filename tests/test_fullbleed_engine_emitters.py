from __future__ import annotations

from pathlib import Path

import pytest

import fullbleed
import fullbleed_assets


def _require_pdf_engine() -> None:
    if not hasattr(fullbleed, "PdfEngine"):
        pytest.skip("fullbleed native extension is not available in this test environment")


def _inter_font_path() -> str:
    return str(fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"))


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
    engine.document_css_href = "styles/engine.css"
    engine.document_css_media = "print"
    engine.document_css_required = True
    meta = engine.document_metadata()

    assert meta["document_lang"] == "fr-CA"
    assert meta["document_title"] == 'Updated "Title" <x&y>'
    assert meta["document_css_href"] == "styles/engine.css"
    assert meta["document_css_media"] == "print"
    assert meta["document_css_required"] is True

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
    assert '<link rel="stylesheet" href="styles/engine.css" media="print" />' in html_text
    assert body_html in html_text
    assert result["css_link_href"] == "styles/engine.css"
    assert result["css_link_media"] == "print"
    assert result["css_link_preexisting"] is False


def test_pdf_engine_document_metadata_properties_accept_none(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Alpha")
    engine.document_lang = None
    engine.document_title = None
    engine.document_css_href = None
    engine.document_css_source_path = None
    engine.document_css_media = None
    engine.document_css_required = False

    assert engine.document_lang is None
    assert engine.document_title is None
    assert engine.document_css_href is None
    assert engine.document_css_source_path is None
    assert engine.document_css_media is None
    assert engine.document_css_required is False

    html_path = tmp_path / "raw.html"
    css_path = tmp_path / "raw.css"
    engine.emit_artifacts("<div>x</div>", "body{}", str(html_path), str(css_path))
    html_text = html_path.read_text(encoding="utf-8")
    assert '<html lang="en">' in html_text
    assert "<title>fullbleed document</title>" in html_text


def test_pdf_engine_pdfua1_profile_emits_pdfua_metadata_and_tags() -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="ua",
        font_files=[_inter_font_path()],
        document_lang="en-US",
        document_title="PDF/UA Alias",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main><p>Tagged payload</p></main></body></html>",
        "body{font-family:Inter}",
    )

    assert b'pdfuaid:part="1"' in pdf
    assert b"/StructTreeRoot" in pdf
    assert b"/MarkInfo << /Marked true >>" in pdf
    assert b"/Lang (en-US)" in pdf
    assert b"/FontFile" in pdf


def test_pdf_engine_pdfua1_profile_requires_embedded_fonts() -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="ua",
        document_lang="en-US",
        document_title="PDF/UA Alias",
    )
    with pytest.raises(ValueError, match="pdfua1 requires"):
        engine.render_pdf(
            "<!doctype html><html><body><main><p>Tagged payload</p></main></body></html>",
            "body{font-family:Helvetica}",
        )


def test_pdf_engine_pdfua2_profile_emits_pdf20_pdfua2_metadata_and_tags() -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="pdf/ua-2",
        font_files=[_inter_font_path()],
        document_lang="en-US",
        document_title="PDF/UA-2 Alias",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main><p>Tagged PDF/UA-2 payload</p></main></body></html>",
        "body{font-family:Inter}",
    )

    assert pdf.startswith(b"%PDF-2.0")
    assert b'pdfuaid:part="2"' in pdf
    assert b'pdfuaid:rev="2024"' in pdf
    assert b"/StructTreeRoot" in pdf
    assert b"/S /Document" in pdf
    assert b"/NS (http://iso.org/pdf2/ssn)" in pdf
    assert b"/MarkInfo << /Marked true >>" in pdf
    assert b"/Lang (en-US)" in pdf
    assert b"/FontFile" in pdf


@pytest.mark.parametrize(
    ("profile", "claim", "declaration"),
    [
        ("wt1r", "wtpdf1r", b"http://pdfa.org/declarations/wtpdf#reuse1.0"),
        ("wt1a", "wtpdf1a", b"http://pdfa.org/declarations/wtpdf#accessibility1.0"),
    ],
)
def test_pdf_engine_wtpdf_profiles_emit_pdf_declarations_and_tags(
    tmp_path: Path, profile: str, claim: str, declaration: bytes
) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile=profile,
        font_files=[_inter_font_path()],
        document_lang="en-US",
        document_title="WTPDF Alias",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main><p>Well-tagged payload</p></main></body></html>",
        "body{font-family:Inter}",
    )

    assert pdf.startswith(b"%PDF-2.0")
    assert b"<pdfd:declarations>" in pdf
    assert declaration in pdf
    assert b"/StructTreeRoot" in pdf
    assert b"/S /Document" in pdf
    assert b"/NS (http://iso.org/pdf2/ssn)" in pdf
    assert b"/MarkInfo << /Marked true >>" in pdf
    assert b"/Lang (en-US)" in pdf
    assert b"/FontFile" in pdf

    out = tmp_path / f"{claim}.pdf"
    out.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(out))
    assert claim in report["profile"]["claims"]
    assert report["profile"]["pdf_declaration_present"] is True
    assert report["profile"]["struct_tree_root_present"] is True
    assert report["profile"]["mark_info_present"] is True
    assert report["profile"]["lang_present"] is True
    assert report["profile"]["seed_blockers"] == []


def test_pdf_engine_pdfa4_profile_emits_pdf20_and_pdfa4_metadata(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="pdf/a-4",
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main></main></body></html>",
        "body{}",
    )

    assert pdf.startswith(b"%PDF-2.0")
    assert b'pdfaid:part="4"' in pdf
    assert b'pdfaid:rev="2020"' in pdf
    assert b"pdfaid:conformance" not in pdf

    out = tmp_path / "pdfa4.pdf"
    out.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(out))
    assert "pdfa4" in report["profile"]["claims"]
    assert report["profile"]["metadata_present"] is True
    assert report["profile"]["output_intent_present"] is True
    assert report["profile"]["seed_blockers"] == []


@pytest.mark.parametrize(
    ("profile", "conformance", "embedded_files"),
    [
        ("pdf/a-4e", b'pdfaid:conformance="E"', False),
        ("pdf/a-4f", b'pdfaid:conformance="F"', True),
    ],
)
def test_pdf_engine_pdfa4_conformance_profiles_emit_required_profile_markers(
    tmp_path: Path, profile: str, conformance: bytes, embedded_files: bool
) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile=profile,
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main></main></body></html>",
        "body{}",
    )

    assert pdf.startswith(b"%PDF-2.0")
    assert b'pdfaid:part="4"' in pdf
    assert b'pdfaid:rev="2020"' in pdf
    assert conformance in pdf
    if embedded_files:
        assert b"/EmbeddedFiles" in pdf
        assert b"/Type /Filespec" in pdf
        assert b"/Type /EmbeddedFile" in pdf
        assert b"/AFRelationship /Data" in pdf
        assert b"/AF [" in pdf

    out = tmp_path / f"{profile.replace('/', '_')}.pdf"
    out.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(out))
    canonical = profile.replace("pdf/a-", "pdfa")
    assert canonical in report["profile"]["claims"]
    assert ("pdfa4" in report["profile"]["claims"]) is False
    assert report["profile"]["metadata_present"] is True
    assert report["profile"]["output_intent_present"] is True
    assert report["profile"]["embedded_files_present"] is embedded_files
    assert report["profile"]["seed_blockers"] == []


@pytest.mark.parametrize(
    ("profile", "part", "conformance"),
    [
        ("pdf/a-2u", b'pdfaid:part="2"', b'pdfaid:conformance="U"'),
        ("pdf/a-3u", b'pdfaid:part="3"', b'pdfaid:conformance="U"'),
    ],
)
def test_pdf_engine_pdfau_profiles_emit_unicode_conformance_metadata(
    tmp_path: Path, profile: str, part: bytes, conformance: bytes
) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile=profile,
        font_files=[_inter_font_path()],
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main><p>Unicode mapped payload</p></main></body></html>",
        "body{font-family:Inter}",
    )

    assert part in pdf
    assert conformance in pdf
    assert b"/ToUnicode" in pdf

    out = tmp_path / f"{profile.replace('/', '_')}.pdf"
    out.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(out))
    canonical = profile.replace("pdf/a-", "pdfa")
    assert canonical in report["profile"]["claims"]
    assert "pdfa2b" not in report["profile"]["claims"]
    assert "pdfa3b" not in report["profile"]["claims"]
    assert report["profile"]["output_intent_present"] is True
    assert report["profile"]["seed_blockers"] == []


@pytest.mark.parametrize(
    ("profile", "part"),
    [
        ("pdf/a-1a", b'pdfaid:part="1"'),
        ("pdf/a-2a", b'pdfaid:part="2"'),
        ("pdf/a-3a", b'pdfaid:part="3"'),
    ],
)
def test_pdf_engine_pdfaa_profiles_emit_tagged_pdfa_metadata(
    tmp_path: Path, profile: str, part: bytes
) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile=profile,
        font_files=[_inter_font_path()],
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
        document_lang="en-US",
        document_title="PDF/A A-level seed",
    )
    pdf = engine.render_pdf(
        "<!doctype html><html><body><main><p>Tagged A-level payload</p></main></body></html>",
        "body{font-family:Inter}",
    )

    assert part in pdf
    assert b'pdfaid:conformance="A"' in pdf
    assert b"/StructTreeRoot" in pdf
    assert b"/MarkInfo << /Marked true >>" in pdf
    assert b"/Lang (en-US)" in pdf

    out = tmp_path / f"{profile.replace('/', '_')}.pdf"
    out.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(out))
    canonical = profile.replace("pdf/a-", "pdfa")
    assert canonical in report["profile"]["claims"]
    assert report["profile"]["struct_tree_root_present"] is True
    assert report["profile"]["mark_info_present"] is True
    assert report["profile"]["lang_present"] is True
    assert report["profile"]["seed_blockers"] == []


def test_pdf_engine_pdfvt1_profile_requires_output_intent() -> None:
    _require_pdf_engine()

    with pytest.raises(ValueError, match="pdfvt1 requires output_intent"):
        fullbleed.PdfEngine(pdf_profile="vt")


def test_pdf_engine_pdfvt1_profile_emits_deterministic_identifiers() -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="pdf/vt",
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
    )
    html = "<!doctype html><html><body><main></main></body></html>"
    css = "body{font-family:Helvetica}"
    first = engine.render_pdf(html, css)
    second = engine.render_pdf(html, css)

    assert first == second
    assert b'pdfvtid:GTS_PDFVTVersion="PDF/VT-1"' in first
    assert b'pdfvtid:GTS_PDFVTModDate="1970-01-01T00:00:00Z"' in first
    assert b"/GTS_PDFVTVersion (PDF/VT-1)" in first


def test_inspect_pdf_reports_profile_seed_markers(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(
        pdf_profile="pdf/vt",
        output_intent_icc="data:application/octet-stream;base64,AAAA",
        output_intent_identifier="sRGB IEC61966-2.1",
        output_intent_info="sRGB",
    )
    out = tmp_path / "profile.pdf"
    engine.render_pdf_to_file(
        "<!doctype html><html><body><main></main></body></html>",
        "body{}",
        str(out),
    )
    report = fullbleed.inspect_pdf(str(out))

    assert "pdfvt1" in report["profile"]["claims"]
    assert "pdfx4" in report["profile"]["claims"]
    assert report["profile"]["metadata_present"] is True
    assert report["profile"]["output_intent_present"] is True
    assert report["profile"]["dpart_root_present"] is True
    assert report["profile"]["dpart_present"] is True
    assert report["profile"]["page_dpart_present"] is True
    assert report["profile"]["pdfvt_dpart_root_node_valid"] is True
    assert report["profile"]["pdfvt_dpart_parent_valid"] is True
    assert report["profile"]["pdfvt_dpart_node_name_list_valid"] is True
    assert report["profile"]["pdfvt_dpart_leaf_valid"] is True
    assert report["profile"]["pdfvt_dpart_page_range_valid"] is True
    assert report["profile"]["pdfvt_dpart_graph_valid"] is True
    assert report["profile"]["pdfvt_mod_date_matches_xmp"] is True
    assert report["profile"]["seed_blockers"] == []


def test_pdf_engine_css_required_fails_without_href(tmp_path: Path) -> None:
    _require_pdf_engine()

    engine = fullbleed.PdfEngine(document_lang="en-US", document_title="Strict CSS")
    engine.document_css_required = True

    html_path = tmp_path / "strict.html"
    with pytest.raises(ValueError):
        engine.emit_html("<div>x</div>", str(html_path), True)
