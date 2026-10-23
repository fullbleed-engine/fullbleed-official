"""Semantic effect forms retain real content ownership for each placement.

Object-level checks complement independent reader and veraPDF verification.
They do not establish general accessibility or assistive-technology behavior.
"""
import io
import os
from pathlib import Path

import fullbleed
import fullbleed_assets
from pypdf import PdfReader
from pypdf.generic import ArrayObject, ContentStream, IndirectObject
import pytest


CSS = "@page{size:300pt 400pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}h1,h2{font-weight:normal}p{margin:0 0 12pt}"
CASES = {
    "target": '<h1>Report</h1><p><a href="#target">Details</a></p><h2 id="target" style="opacity:0.6">Target heading</h2>',
    "link": '<h1>Report</h1><p><a href="https://example.invalid/report" style="display:inline-block;opacity:0.6">Read report</a></p>',
    "nested": '<h1>Report</h1><p><a href="#target">Details</a></p><div style="opacity:0.7"><h2 id="target">Target heading</h2><div style="opacity:0.5"><p>Nested text</p><p><a href="https://example.invalid/report">Read report</a></p></div></div>',
    "inline": '<div>Before <span style="opacity:0.6">Rich text <a href="https://example.invalid/report">Read report</a></span> after.</div>',
    "anonymous": '<div>Before <span style="opacity:0.6">First <div>Block inside an inline</div> Last</span> After</div>',
    "quote": '<blockquote>Before <span style="opacity:0.6">Rich quote</span> after.</blockquote>',
    "block-span": '<div><span style="display:block">Before <span style="opacity:0.6">Rich block</span> after.</span></div>',
}


def ref_id(value):
    return value.idnum if isinstance(value, IndirectObject) else value.indirect_reference.idnum


def check_form_ownership(document):
    root = document.trailer["/Root"]["/StructTreeRoot"]
    nums = root["/ParentTree"]["/Nums"]
    keys = [int(nums[i]) for i in range(0, len(nums), 2)]
    assert keys == sorted(set(keys)), "Parent-tree keys must be unique and ordered"
    parents = {int(nums[i]): nums[i + 1] for i in range(0, len(nums), 2)}
    elements = {}
    references = {}

    def visit(value, owner=None):
        value = value.get_object()
        if isinstance(value, ArrayObject):
            for child in value:
                visit(child, owner)
        elif hasattr(value, "get"):
            if value.get("/Type") == "/StructElem":
                key = ref_id(value)
                assert key not in elements
                elements[key] = value
                visit(value.get("/K", ArrayObject()), key)
            elif value.get("/Type") == "/MCR":
                stream = value["/Stm"]
                key = (ref_id(stream), int(value["/MCID"]))
                assert key not in references, "Each marked-content sequence has one owner"
                references[key] = (owner, ref_id(value.raw_get("/Pg")))
                assert ref_id(parents[int(stream["/StructParents"])][key[1]]) == owner

    visit(root["/K"])
    seen_forms = set()
    stream_keys = set()
    seen_references = set()

    def walk(stream, resources, page, active=()):
        identity = ref_id(stream) if getattr(stream, "indirect_reference", None) else None
        assert identity not in active
        next_active = (*active, identity) if identity else active
        semantic = stream.get("/StructParents")
        if semantic is not None:
            assert identity not in seen_forms, "Repeated semantic placements need distinct ownership"
            seen_forms.add(identity)
            assert int(semantic) not in stream_keys
            stream_keys.add(int(semantic))
        mcids = []
        for operands, operator in ContentStream(stream, document).operations:
            if operator == b"BDC" and hasattr(operands[1], "get") and "/MCID" in operands[1]:
                mcid = int(operands[1]["/MCID"])
                if semantic is not None:
                    key = (identity, mcid)
                    assert references[key][1] == page
                    seen_references.add(key)
                    mcids.append(mcid)
            elif operator == b"Do":
                child = resources["/XObject"][operands[0]]
                if child.get("/Subtype") == "/Form":
                    walk(child, child.get("/Resources", resources), page, next_active)
        if semantic is not None:
            assert mcids == list(range(len(mcids))), "MCIDs are scoped to each form stream"

    for page in document.pages:
        key = int(page["/StructParents"])
        assert key not in stream_keys
        stream_keys.add(key)
        walk(page.get_contents(), page["/Resources"], ref_id(page))
        for reference in page.get("/Annots", []):
            annotation = reference.get_object()
            key = int(annotation["/StructParent"])
            assert key not in stream_keys
            stream_keys.add(key)
            assert elements[ref_id(parents[key])]["/S"] == "/Link"
            if "/SD" in annotation.get("/A", {}):
                target = annotation["/A"]["/SD"][0].get_object()
                assert target["/S"] == "/H2"
                assert ref_id(target.raw_get("/Pg")) == ref_id(page)
    assert seen_forms and references
    assert seen_references == set(references), "Every form MCR must identify painted content"
    return len(seen_forms)


@pytest.mark.parametrize("profile", ["tagged", "pdfua1", "pdfua2"])
@pytest.mark.parametrize("copies", [1, 2])
@pytest.mark.parametrize("case", list(CASES))
def test_effect_form_ownership_survives_compilation_and_repeated_documents(profile, copies, case):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Semantic opacity forms",
        font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))],
    )
    compiled = engine.compile_pdf(CASES[case], CSS)
    pdf = bytes(compiled.render_pdf_batch(copies))
    assert pdf == bytes(compiled.render_pdf_batch(copies))
    document = PdfReader(io.BytesIO(pdf), strict=True)
    assert len(document.pages) == copies
    assert check_form_ownership(document) >= copies
    if case in {"inline", "anonymous", "quote", "block-span"}:
        def top_level(value):
            value = value.get_object()
            if isinstance(value, ArrayObject):
                return [node for child in value for node in top_level(child)]
            if value.get("/S") == "/Document":
                return top_level(value["/K"])
            return [value]

        children = top_level(document.trailer["/Root"]["/StructTreeRoot"]["/K"])
        expected = "/BlockQuote" if case == "quote" else "/P"
        assert all(child.get_object()["/S"] == expected for child in children)
        assert len(children) == (3 if case == "anonymous" else 1) * copies
    if directory := os.environ.get("FULLBLEED_FORM_TEST_OUTPUT"):
        path = Path(directory)
        path.mkdir(parents=True, exist_ok=True)
        (path / f"{case}-{profile}-{copies}.pdf").write_bytes(pdf)


@pytest.mark.parametrize("profile", ["none", "tagged", "pdfua1", "pdfua2"])
@pytest.mark.parametrize("compression", ["throughput", "compact"])
@pytest.mark.parametrize("html", [
    '<h1>Report</h1><div style="opacity:0.6"><p>{{name}}</p></div>',
    '<h1>Report</h1><p>Before <span style="opacity:0.6">{{name}}</span> after.</p>',
    '<h1>Report</h1><p>Before <span style="opacity:0.6">{{name}} <span style="opacity:0.5">details</span></span> after.</p>',
])
def test_effect_forms_use_each_reflow_recipient_content(profile, compression, html):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US", document_title="Recipient forms",
        font_files=[str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))],
    )
    compiled = engine.compile_pdf(html, CSS)
    def render():
        return bytes(compiled.render_pdf_reflow_bindings({"name": ["ALPHA", "BRAVO"]}, compression=compression))
    pdf = render()
    assert pdf == render()
    document = PdfReader(io.BytesIO(pdf), strict=True)
    assert len(document.pages) == 2
    for page, expected, absent in zip(document.pages, ["ALPHA", "BRAVO"], ["BRAVO", "ALPHA"]):
        text = page.extract_text()
        assert expected in text and absent not in text
    if profile != "none":
        expected = 4 if 'opacity:0.5' in html else 2
        assert check_form_ownership(document) == expected
