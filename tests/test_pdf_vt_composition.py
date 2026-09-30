import copy
from pathlib import Path

import fullbleed
import pytest

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "python/fullbleed_assets/fonts/NotoSans-Regular.ttf"
CSS = "@page{size:200pt 200pt;margin:15pt} body{font-family:'Noto Sans';font-size:10pt}"
JOB = {"id": "September statements", "metadata": {"product": "statements", "copies": 1}, "records": [
    {"id": "000001", "metadata": {"postal_code": "60601", "recipient": "Zoë", "duplex": True},
     "documents": [{"id": "statement", "metadata": {"media": "body"}}, {"id": "insert"}]},
    {"id": "000002", "metadata": {"finishing": {"binding": "staple", "positions": [1, 2]}},
     "documents": [{"id": "statement"}]},
]}


def runtime(icc, job=None):
    return fullbleed.PdfEngine(pdf_profile="pdfvt1", output_intent_icc=icc,
        document_title="VDP job", document_timestamp="2026-09-30T00:00:00Z",
        pdf_vt_job=job, font_files=[str(FONT)])


def report(pdf, tmp_path):
    path = tmp_path / "job.pdf"
    path.write_bytes(pdf)
    result = fullbleed.inspect_pdf(str(path))
    assert result["profile"]["seed_blockers"] == [], result
    return result["profile"]


def pages(count):
    return "<html><body>" + "".join(
        "<div style='height:20pt;background:red;" + ("break-before:page" if i else "") + "'></div>"
        for i in range(count)) + "</body></html>"


@pytest.mark.parametrize("method", [
    "render_pdf_batch", "render_pdf_batch_parallel", "render_pdf_batch_with_css",
    "render_pdf_batch_to_file", "render_pdf_batch_to_file_parallel",
    "render_pdf_batch_to_file_parallel_with_page_data", "render_pdf_batch_with_css_to_file",
])
def test_recipient_documents_and_metadata_follow_final_pagination(print_icc_uri, tmp_path, method):
    engine = runtime(print_icc_uri, JOB)
    def render():
        html = [pages(2), pages(1), pages(3)]
        arguments = ([(source, CSS) for source in html],) if "with_css" in method else (html, CSS)
        if "to_file" in method:
            path = tmp_path / "streamed.pdf"
            getattr(engine, method)(*arguments, str(path))
            return path.read_bytes()
        return getattr(engine, method)(*arguments)
    pdf = render()
    data = report(pdf, tmp_path)
    assert data["pdfvt_record_level"] == 1
    assert data["pdfvt_record_count"] == 2
    assert data["pdfvt_document_count"] == 3
    assert data["pdfvt_dpm_node_count"] == 6
    parts = data["pdfvt_parts"]
    assert [(p["name"], p["id"]) for p in parts] == [
        ("Job", "September statements"), ("Record", "000001"), ("Document", "statement"),
        ("Document", "insert"), ("Record", "000002"), ("Document", "statement")]
    assert [(p["first_page"], p["last_page"]) for p in parts if p["name"] == "Document"] == [(1, 2), (3, 3), (4, 6)]
    assert parts[1]["metadata"]["Fullbleed"]["Metadata"] == JOB["records"][0]["metadata"]
    assert parts[4]["metadata"]["Fullbleed"]["Metadata"] == JOB["records"][1]["metadata"]
    assert pdf == render()


@pytest.mark.parametrize("mode", ["copies", "fixed", "reflow", "reflow_compact"])
def test_compiled_records_receive_distinct_parts(print_icc_uri, tmp_path, mode):
    engine = runtime(print_icc_uri)
    compiled = engine.compile_pdf("<p>{{name}}</p>", CSS)
    if mode == "copies":
        pdf = compiled.render_pdf_batch(3)
    elif mode == "fixed":
        pdf = compiled.render_pdf_bindings({"name": ["Alice", "Bob", "Charlie"]})
    else:
        pdf = compiled.render_pdf_reflow_bindings({"name": ["Alice", "Bob " * 900, "Charlie"]},
            compression="compact" if mode == "reflow_compact" else "throughput")
    data = report(pdf, tmp_path)
    assert data["pdfvt_record_count"] == 3
    leaves = [p for p in data["pdfvt_parts"] if p["name"] == "Document"]
    assert len(leaves) == 3
    assert leaves[0]["first_page"] == 1
    assert leaves[1]["first_page"] == leaves[0]["last_page"] + 1
    assert leaves[2]["first_page"] == leaves[1]["last_page"] + 1
    if mode.startswith("reflow"):
        assert leaves[1]["last_page"] > leaves[1]["first_page"]


def test_declared_compiled_records_and_metadata_order_are_deterministic(print_icc_uri, tmp_path):
    job = copy.deepcopy(JOB)
    first = runtime(print_icc_uri, job).compile_pdf("<p>{{name}}</p>", CSS)
    job["metadata"] = dict(reversed(list(job["metadata"].items())))
    second = runtime(print_icc_uri, job).compile_pdf("<p>{{name}}</p>", CSS)
    bindings = {"name": ["statement", "insert", "statement"]}
    pdf = first.render_pdf_bindings(bindings)
    assert pdf == second.render_pdf_bindings(bindings)
    assert report(pdf, tmp_path)["pdfvt_record_count"] == 2


@pytest.mark.parametrize("count", [1, 2, 4])
def test_declared_input_count_must_match_rendered_job(print_icc_uri, count):
    engine = runtime(print_icc_uri, JOB)
    with pytest.raises(Exception, match="PDF_VT_JOB_INVALID: .*input documents"):
        engine.render_pdf_batch([pages(1)] * count, CSS)


@pytest.mark.parametrize("job", [
    {"id": ""}, {"id": "job", "typo": True},
    {"id": "job", "metadata": {"bad": None}},
    {"id": "job", "metadata": {"bad": float("nan")}},
    {"id": "job", "metadata": {"bad": 1e100}},
    {"id": "job", "metadata": {"bad": 2 ** 80}},
    {"id": "job", "records": [{"id": "record", "documents": []}]},
    {"id": "job", "records": [{"id": "record", "documents": [{"id": "same"}, {"id": "same"}]}]},
])
def test_invalid_job_metadata_fails_before_output(print_icc_uri, job):
    with pytest.raises(ValueError, match="PDF_VT_JOB_INVALID"):
        runtime(print_icc_uri, job)


def test_reusable_opaque_images_carry_scope_and_encapsulation_hints(print_icc_uri, tmp_path):
    # Small opaque RGB PNG; the duplicate resource must be shared across records.
    import base64
    import struct
    import zlib
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00")) + chunk(b"IEND", b"")
    html = '<img src="data:image/png;base64,' + base64.b64encode(png).decode() + '" width="20" height="20">'
    pdf = runtime(print_icc_uri).render_pdf_batch([html, html], CSS)
    data = report(pdf, tmp_path)
    assert data["pdfvt_reuse_hints_valid"] is True
    assert data["pdfvt_reuse_hint_count"] == 1
    assert data["pdfvt_encapsulated_xobject_count"] == 1
    assert b"/GTS_Scope /File /GTS_Encapsulated true" in pdf


def test_dpm_reals_and_names_are_data_not_pdf_actions(print_icc_uri, tmp_path):
    job = {"id": "job", "metadata": {"JS": "private production label", "Ref": "order-17", "large": 1e20, "fraction": 0.25}}
    pdf = runtime(print_icc_uri, job).render_pdf(pages(1), CSS)
    metadata = report(pdf, tmp_path)["pdfvt_parts"][0]["metadata"]["Fullbleed"]["Metadata"]
    assert metadata["large"] == pytest.approx(1e20)
    assert metadata["fraction"] == 0.25
    assert metadata["JS"] == "private production label"


def test_mcp_compilation_preserves_declared_vt_job(print_icc_uri, tmp_path):
    import base64
    import shutil
    from fullbleed_cli.mcp import FullbleedMcpServer
    (tmp_path / "intent.icc").write_bytes(base64.b64decode(print_icc_uri.split(",", 1)[1]))
    shutil.copyfile(FONT, tmp_path / "font.ttf")
    server = FullbleedMcpServer(tmp_path)
    compiled = server._compile({"html": "<p>{{name}}</p>", "css": CSS, "pdf_profile": "pdfvt1",
        "document_title": "MCP job", "document_timestamp": "2026-09-30T00:00:00Z",
        "output_intent_icc_path": "intent.icc", "font_paths": ["font.ttf"], "pdf_vt_job": JOB})
    result = server._render_compiled({"compile_id": compiled["compile_id"], "mode": "fixed_bindings",
        "bindings": {"name": ["Alice", "Insert", "Bob"]}, "output_path": "job.pdf"})
    assert result["ok"] is True
    assert report((tmp_path / "job.pdf").read_bytes(), tmp_path)["pdfvt_record_count"] == 2
