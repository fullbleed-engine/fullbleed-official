"""Installed writer contract, identity replay and negative artifact checks."""
import re
from datetime import datetime, timezone

import fullbleed
import pytest
from fullbleed_cli import cli

DATE = "2026-09-30T12:34:56Z"
HTML = "<html><body><div style='width:20pt;height:20pt;background:#ff0000'></div></body></html>"


def engine(icc, **options):
    arguments = dict(pdf_profile="pdfvt1", output_intent_icc=icc,
                     document_title="Crédit & résumé 東京", document_timestamp=DATE)
    arguments.update(options)
    return fullbleed.PdfEngine(**arguments)


def xmp(pdf, name):
    return re.search(rb'\b' + name.encode() + rb'="([^"]*)"', pdf).group(1)


@pytest.mark.parametrize("profile", ["pdfx4", "pdfvt1"])
@pytest.mark.parametrize("requested", ["1.6", "1.7", "2.0"])
def test_print_profile_forces_pdf16_and_coherent_identity(print_icc_uri, tmp_path, profile, requested):
    runtime = engine(print_icc_uri, pdf_profile=profile, pdf_version=requested)
    pdf = runtime.render_pdf(HTML, "")
    assert pdf.startswith(b"%PDF-1.6\n")
    for field in ["xmp:CreateDate", "xmp:ModifyDate", "xmp:MetadataDate"]:
        assert xmp(pdf, field) == DATE.encode()
    assert b"/CreationDate (D:20260930123456Z) /ModDate (D:20260930123456Z)" in pdf
    assert xmp(pdf, "pdf:Trapped") == b"False"
    assert xmp(pdf, "xmpMM:VersionID") == b"1"
    assert xmp(pdf, "xmpMM:RenditionClass") == b"default"
    path = tmp_path / "print.pdf"
    path.write_bytes(pdf)
    report = fullbleed.inspect_pdf(str(path))["profile"]
    assert report["pdfx_contract_valid"] is True, report
    assert report["seed_blockers"] == []
    assert report["document_timestamp"] == DATE
    assert runtime.document_timestamp == DATE


def test_content_identity_replay_and_distinct_writes(print_icc_uri):
    runtime = engine(print_icc_uri)
    first = runtime.render_pdf(HTML, "")
    assert first == runtime.render_pdf(HTML, "")
    changed = runtime.render_pdf(HTML.replace("#ff0000", "#00ff00"), "")
    assert xmp(first, "xmpMM:DocumentID") != xmp(changed, "xmpMM:DocumentID")
    later = engine(print_icc_uri, document_timestamp="2026-10-01T12:34:56Z").render_pdf(HTML, "")
    assert xmp(first, "xmpMM:DocumentID") == xmp(later, "xmpMM:DocumentID")
    assert xmp(first, "xmpMM:InstanceID") != xmp(later, "xmpMM:InstanceID")
    assert first != later


@pytest.mark.parametrize("options, message", [
    ({"document_title": None}, "non-empty document title"),
    ({"document_title": " \t"}, "non-empty document title"),
    ({"document_title": "bad\x01title"}, "XML 1.0"),
    ({"document_timestamp": None}, "explicit document_timestamp"),
    ({"document_timestamp": "2026-02-29T00:00:00Z"}, "PDF_TIMESTAMP_INVALID"),
    ({"output_intent_icc": "data:application/octet-stream;base64,AAAA"}, "ICC_PROFILE_INVALID"),
    ({"output_intent_components": 4}, "components do not match"),
])
def test_print_contract_rejects_bad_inputs(print_icc_uri, options, message):
    with pytest.raises(ValueError, match=message):
        engine(print_icc_uri, **options)


def test_source_date_epoch_and_explicit_clock(print_icc_uri, monkeypatch):
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "0")
    assert engine(print_icc_uri, document_timestamp="source-date-epoch").document_timestamp == "1970-01-01T00:00:00Z"
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "-1")
    with pytest.raises(ValueError, match="SOURCE_DATE_EPOCH"):
        engine(print_icc_uri, document_timestamp="source-date-epoch")
    monkeypatch.delenv("SOURCE_DATE_EPOCH")
    with pytest.raises(ValueError, match="SOURCE_DATE_EPOCH"):
        engine(print_icc_uri, document_timestamp="source-date-epoch")
    before = datetime.now(timezone.utc).replace(microsecond=0)
    current = engine(print_icc_uri, document_timestamp="current").document_timestamp
    assert before <= datetime.fromisoformat(current) <= datetime.now(timezone.utc)


@pytest.mark.parametrize("before, after, blocker", [
    (b"%PDF-1.6", b"%PDF-1.7", "pdfx_requires_pdf16"),
    (b'xmpMM:DocumentID=', b'xmpMM:DocumxntID=', "pdfx_missing_DocumentID"),
    (b'/Trapped /False', b'/Trapped /True ', "pdfx_info_trapped_mismatch"),
    (b'/CreationDate (D:20260930123456Z)', b'/CreationDate (D:20260930123457Z)', "pdfx_info_date_mismatch"),
    (b'/TrimBox [0 0 ', b'/TrimBox [9 9 ', None),
])
def test_inspector_detects_tampered_artifacts(print_icc_uri, tmp_path, before, after, blocker):
    pdf = engine(print_icc_uri).render_pdf(HTML, "")
    assert before in pdf
    changed = pdf.replace(before, after, 1)
    assert len(changed) == len(pdf)
    path = tmp_path / "changed.pdf"
    path.write_bytes(changed)
    report = fullbleed.inspect_pdf(str(path))["profile"]
    if blocker:
        assert blocker in report["seed_blockers"], report
        assert report["pdfx_contract_valid"] is False
    else:
        # A smaller legal TrimBox must stay valid.
        assert report["pdfx_contract_valid"] is True


def test_cli_manifest_retains_resolved_timestamp(print_icc_uri, monkeypatch):
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "0")
    args = cli._build_parser().parse_args([
        "render", "--html-str", HTML, "--out", "unused.pdf", "--pdf-profile", "vt",
        "--document-title", "Job", "--timestamp-source", "SOURCE_DATE_EPOCH",
        "--output-intent-icc", print_icc_uri,
    ])
    cli._build_engine(args)
    assert cli._build_manifest(args)["pdf"]["document_timestamp"] == "1970-01-01T00:00:00Z"


def test_cli_written_manifest_keeps_resolved_date(print_icc_uri, monkeypatch, tmp_path, capsys):
    import json
    from fullbleed_cli import cli
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "0")
    manifest = tmp_path / "manifest.json"
    code = cli.main(["--json", "render", "--html-str", "<div style='height:12pt;background:red'></div>",
        "--out", str(tmp_path / "print.pdf"), "--pdf-profile", "pdfvt1",
        "--document-title", "Job", "--output-intent-icc", print_icc_uri,
        "--timestamp-source", "SOURCE_DATE_EPOCH", "--emit-manifest", str(manifest)])
    assert code == 0, capsys.readouterr()
    assert json.loads(manifest.read_text())["pdf"]["document_timestamp"] == "1970-01-01T00:00:00Z"
