"""Imported PDF features keep their format and extension declarations."""
import fullbleed
from pypdf import PdfReader, PdfWriter
from pypdf.generic import (
    ArrayObject, ByteStringObject, DecodedStreamObject, DictionaryObject,
    FloatObject, IndirectObject, NameObject, NumberObject, TextStringObject,
)
import pytest


def fixture(path, header, *, catalog_version=None, extensions=None, transparent=False, utf8=False):
    writer = PdfWriter()
    writer.pdf_header = ("%PDF-" + header).encode("ascii")
    page = writer.add_blank_page(width=200, height=200)
    if catalog_version is not None:
        writer._root_object[NameObject("/Version")] = (
            NameObject("/" + catalog_version) if type(catalog_version) is str
            else catalog_version)
    if extensions is not None:
        writer._root_object[NameObject("/Extensions")] = extensions
    content = DecodedStreamObject()
    content.set_data(b"q " + (b"/Alpha gs " if transparent else b"") + b"0 0 1 rg 20 20 50 30 re f Q\n")
    page[NameObject("/Contents")] = writer._add_object(content)
    if transparent:
        page[NameObject("/Resources")] = DictionaryObject({NameObject("/ExtGState"): DictionaryObject({
            NameObject("/Alpha"): DictionaryObject({NameObject("/Type"): NameObject("/ExtGState"),
                                                   NameObject("/ca"): FloatObject(.4)})})})
    label = path.stem.encode("ascii")
    annotation = DictionaryObject({
        NameObject("/Type"): NameObject("/Annot"), NameObject("/Subtype"): NameObject("/Link"),
        NameObject("/Rect"): ArrayObject([NumberObject(n) for n in (20, 20, 70, 50)]),
        NameObject("/Border"): ArrayObject([NumberObject(0)] * 3),
        NameObject("/Contents"): ByteStringObject((b"\xef\xbb\xbf" if utf8 else b"") + label),
        NameObject("/A"): DictionaryObject({NameObject("/S"): NameObject("/URI"),
            NameObject("/URI"): ByteStringObject(b"https://example.invalid/" + label)}),
    })
    page[NameObject("/Annots")] = ArrayObject([writer._add_object(annotation)])
    with path.open("wb") as stream:
        writer.write(stream)


def render(method, template, overlay, output):
    if method == "stamp":
        return fullbleed.finalize_stamp_pdf(str(template), str(overlay), str(output))
    return fullbleed.finalize_compose_pdf([("template", str(template))],
        [("template", 0, 0, 0., 0.)], str(overlay), str(output))


def raw_string(value):
    return bytes(value) if isinstance(value, bytes) else value.original_bytes


@pytest.mark.parametrize("method", ["stamp", "compose"])
@pytest.mark.parametrize("th,tc,oh,oc,stamp_version,compose_version", [
    ("1.3", None, "1.4", None, "1.4", "1.7"),
    ("1.7", None, "1.7", "2.0", "2.0", "2.0"),
    ("1.3", "2.0", "1.4", None, "2.0", "2.0"),
    ("2.0", None, "1.7", None, "2.0", "2.0"),
    ("2.0", "1.4", "1.7", None, "2.0", "2.0"),
    ("1.7", "1.3", "1.4", "1.6", "1.7", "1.7"),
])
def test_imported_features_preserve_effective_pdf_version(tmp_path, method, th, tc, oh, oc, stamp_version, compose_version):
    template, overlay, output, replay = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf", "replay.pdf")]
    template_utf8, overlay_utf8 = max(th, tc or th) >= "2.0", max(oh, oc or oh) >= "2.0"
    fixture(template, th, catalog_version=tc, utf8=template_utf8)
    fixture(overlay, oh, catalog_version=oc, utf8=overlay_utf8, transparent=True)
    assert render(method, template, overlay, output)["ok"]
    assert render(method, template, overlay, replay)["ok"]
    assert output.read_bytes() == replay.read_bytes()
    expected = stamp_version if method == "stamp" else compose_version
    assert output.read_bytes().startswith(("%PDF-" + expected + "\n").encode())
    result = PdfReader(output, strict=True)
    catalog_version = result.trailer["/Root"].get("/Version")
    assert catalog_version is None or catalog_version == "/" + expected
    annotations = [ref.get_object() for ref in result.pages[0]["/Annots"]]
    assert {raw_string(a["/Contents"]) for a in annotations} == {
        (b"\xef\xbb\xbf" if template_utf8 else b"") + b"template",
        (b"\xef\xbb\xbf" if overlay_utf8 else b"") + b"overlay",
    }
    assert {a["/A"]["/URI"] for a in annotations} == {
        "https://example.invalid/template", "https://example.invalid/overlay"}
    forms = result.pages[0]["/Resources"]["/XObject"]
    alpha = [float(gs.get_object()["/ca"]) for form in forms.values()
             for gs in form.get_object()["/Resources"].get("/ExtGState", {}).values()]
    assert .4 in alpha


def extension(level, *, hex_url=False):
    url = f"https://example.invalid/extension/{level}"
    return DictionaryObject({
        NameObject("/Type"): NameObject("/DeveloperExtensions"),
        NameObject("/BaseVersion"): NameObject("/1.7"),
        NameObject("/ExtensionLevel"): NumberObject(level),
        NameObject("/URL"): ByteStringObject(url.encode()) if hex_url else TextStringObject(url),
        NameObject("/ExtensionRevision"): TextStringObject("2026"),
        NameObject("/Custom"): ArrayObject([NumberObject(7), TextStringObject("retained")]),
    })


def assert_direct(value):
    assert not isinstance(value, IndirectObject)
    if isinstance(value, dict):
        for item in value.values():
            assert_direct(item)
    elif isinstance(value, list):
        for item in value:
            assert_direct(item)


@pytest.mark.parametrize("method", ["stamp", "compose"])
@pytest.mark.parametrize("kind", ["duplicate", "distinct", "array"])
def test_extension_declarations_survive_import_and_keep_distinct_requirements(tmp_path, method, kind):
    template, overlay, output = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf")]
    fixture(template, "1.7", extensions=DictionaryObject({NameObject("/Type"): NameObject("/Extensions"),
        NameObject("/TEST"): extension(1), NameObject("/SOLO"): extension(7)}))
    declaration = (extension(1, hex_url=True) if kind == "duplicate" else extension(2)
                   if kind == "distinct" else ArrayObject([extension(1, hex_url=True), extension(2)]))
    imported = DictionaryObject({NameObject("/TEST"): declaration})
    if kind == "array":
        imported[NameObject("/SINGLE")] = ArrayObject([extension(8)])
    fixture(overlay, "2.0" if kind == "array" else "1.7", extensions=imported)
    render(method, template, overlay, output)
    expected = "1.7" if kind == "duplicate" else "2.0"
    assert output.read_bytes().startswith(("%PDF-" + expected + "\n").encode())
    result = PdfReader(output, strict=True)
    extensions = result.trailer["/Root"].raw_get("/Extensions")
    assert_direct(extensions)
    assert extensions["/Type"] == "/Extensions"
    expected_test = extension(1) if kind == "duplicate" else ArrayObject([extension(1), extension(2)])
    assert extensions["/TEST"] == expected_test
    assert extensions["/SOLO"] == extension(7)
    if kind == "array":
        assert extensions["/SINGLE"] == ArrayObject([extension(8)])


@pytest.mark.parametrize("method", ["stamp", "compose"])
@pytest.mark.parametrize("version", ["bogus", NumberObject(2), TextStringObject("2.0")])
def test_invalid_version_declarations_do_not_replace_output(tmp_path, method, version):
    template, overlay, output = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "keep.pdf")]
    fixture(template, "1.7")
    fixture(overlay, "1.7", catalog_version=version)
    output.write_bytes(b"untouched existing output")
    with pytest.raises(ValueError, match="pdf compose"):
        render(method, template, overlay, output)
    assert output.read_bytes() == b"untouched existing output"


def test_unused_catalog_templates_do_not_change_output_requirements(tmp_path):
    used, unused, overlay, output = [tmp_path / name for name in ("used.pdf", "unused.pdf", "overlay.pdf", "out.pdf")]
    fixture(used, "1.7")
    fixture(unused, "2.0", utf8=True, extensions=DictionaryObject({NameObject("/UNUSED"): extension(1)}))
    fixture(overlay, "1.7")
    result = fullbleed.finalize_compose_pdf([("used", str(used)), ("unused", str(unused))],
        [("used", 0, 0, 0., 0.)], str(overlay), str(output))
    assert result["ok"] and result["pages_written"] == 1
    assert output.read_bytes().startswith(b"%PDF-1.7\n")
    assert "/Extensions" not in PdfReader(output, strict=True).trailer["/Root"]


def test_empty_stamp_map_does_not_import_unused_overlay_requirements(tmp_path):
    template, overlay, output = [tmp_path / name for name in ("template.pdf", "overlay.pdf", "out.pdf")]
    fixture(template, "1.7")
    fixture(overlay, "2.0", utf8=True, extensions=DictionaryObject({NameObject("/UNUSED"): extension(1)}))
    result = fullbleed.finalize_stamp_pdf(str(template), str(overlay), str(output), page_map=[])
    assert result["ok"] and result["pages_written"] == 0
    assert output.read_bytes().startswith(b"%PDF-1.7\n")
    document = PdfReader(output, strict=True)
    assert len(document.pages) == 1 and "/Extensions" not in document.trailer["/Root"]
    assert [raw_string(ref.get_object()["/Contents"]) for ref in document.pages[0]["/Annots"]] == [b"template"]
