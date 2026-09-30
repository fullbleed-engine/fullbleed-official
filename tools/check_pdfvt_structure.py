#!/usr/bin/env python3
"""Check Fullbleed's VT specimens with pypdf, without importing the engine.

This checks the documented writer contract and known source assignments. It
does not implement every ISO 15930/16612 rule or certify PDF/VT conformance.
"""
from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path
import uuid
import xml.etree.ElementTree as ET


SCHEMA = "fullbleed.pdfvt_separate_parser.v1"
NAMESPACES = {
    "xmp": "http://ns.adobe.com/xap/1.0/",
    "mm": "http://ns.adobe.com/xap/1.0/mm/",
    "pdf": "http://ns.adobe.com/pdf/1.3/",
    "dc": "http://purl.org/dc/elements/1.1/",
    "rdf": "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "vt": "http://www.npes.org/pdfvt/ns/id/",
    "px": "http://www.npes.org/pdfx/ns/id/",
}


class ContractFailure(Exception):
    pass


def check_pdf(path: Path, specimen: str, icc_sha256: str | None = None) -> dict:
    import pypdf
    from pypdf.generic import (
        ArrayObject, BooleanObject, DictionaryObject, FloatObject,
        IndirectObject, NameObject, NumberObject, StreamObject, TextStringObject,
    )

    checks = []
    result = {"schema": SCHEMA, "scope": "separate_parser_writer_contract",
              "independent_iso_conformance": False, "parser": "pypdf",
              "parser_version": pypdf.__version__, "pdf": str(path),
              "pdf_sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "checks": checks}

    def require(condition, code):
        if not condition:
            raise ContractFailure(code)
        if code not in checks:
            checks.append(code)

    def obj(value):
        return value.get_object() if isinstance(value, IndirectObject) else value

    def ref(value):
        if not isinstance(value, IndirectObject):
            value = getattr(value, "indirect_reference", None)
        require(isinstance(value, IndirectObject), "indirect_reference")
        return value.idnum, value.generation

    def data(value, depth=0, count=None):
        count = [0] if count is None else count
        count[0] += 1
        require(depth <= 18 and count[0] <= 10010, "dpm_limits")
        value = obj(value)
        if isinstance(value, BooleanObject):
            return value.value
        if isinstance(value, TextStringObject):
            return str(value)
        if isinstance(value, (FloatObject, NumberObject)):
            import math
            require(math.isfinite(float(value)), "dpm_type")
            return float(value) if isinstance(value, FloatObject) else int(value)
        if isinstance(value, ArrayObject):
            return [data(v, depth + 1, count) for v in value]
        if isinstance(value, DictionaryObject) and not isinstance(value, StreamObject):
            return {str(k)[1:]: data(v, depth + 1, count) for k, v in value.items()}
        raise ContractFailure("dpm_type")

    try:
        require(path.stat().st_size <= 10_000_000, "specimen_size")
        reader = pypdf.PdfReader(path, strict=True)
        require(reader.pdf_header == "%PDF-1.6", "pdf_version")
        require(not reader.is_encrypted, "unencrypted")
        root = reader.root_object
        require(root.get("/Version", "/1.6") == "/1.6", "catalog_version")
        xmp = root.get("/Metadata")
        require(isinstance(obj(xmp), StreamObject), "xmp_stream")
        xmp = obj(xmp)
        require("/Filter" not in xmp, "unfiltered_xmp")
        xml_bytes = xmp.get_data()
        require(len(xml_bytes) <= 2_000_000 and b"<!DOCTYPE" not in xml_bytes, "xmp_size_and_dtd")
        xml = ET.fromstring(xml_bytes)

        def property_value(prefix, name):
            key = "{" + NAMESPACES[prefix] + "}" + name
            values = [node.attrib[key] for node in xml.iter() if key in node.attrib]
            for node in xml.iter(key):
                if prefix == "dc" and name == "title":
                    values.extend(item.text or "" for item in node.findall("rdf:Alt/rdf:li", NAMESPACES)
                                  if item.get("{http://www.w3.org/XML/1998/namespace}lang") == "x-default")
                else:
                    values.append(node.text or "")
            require(len(values) == 1 and bool(values[0].strip()), "xmp_" + name)
            return values[0]

        require(property_value("px", "GTS_PDFXVersion") == "PDF/X-4", "pdfx_claim")
        require(property_value("vt", "GTS_PDFVTVersion") == "PDF/VT-1", "pdfvt_claim")
        dates = [property_value("xmp", name) for name in ("CreateDate", "ModifyDate", "MetadataDate")]
        for date in dates:
            require(len(date) == 20, "utc_date")
            datetime.strptime(date, "%Y-%m-%dT%H:%M:%SZ")
        require(len(set(dates)) == 1, "fresh_write_dates")
        require(property_value("vt", "GTS_PDFVTModDate") == dates[1], "vt_date_agreement")
        for name in ("DocumentID", "InstanceID"):
            value = property_value("mm", name)
            require(value.startswith("uuid:") and str(uuid.UUID(value[5:])) == value[5:], "identity_uuid")
        require(property_value("mm", "VersionID") == "1", "identity_version")
        require(property_value("mm", "RenditionClass") == "default", "identity_rendition")
        info = reader.metadata
        require(info is not None and str(info.get("/Title")) == property_value("dc", "title"), "title_agreement")
        require(str(info.get("/Trapped")) == "/" + property_value("pdf", "Trapped"), "trapping_agreement")
        pdf_date = "D:" + "".join(c for c in dates[0] if c.isdigit()) + "Z"
        require(info.get("/CreationDate") == info.get("/ModDate") == pdf_date, "info_dates")
        ids = reader.trailer.get("/ID", [])
        require(len(ids) == 2 and all(len(v.original_bytes) == 16 for v in ids), "trailer_ids")
        intents = [obj(i) for i in obj(root.get("/OutputIntents", [])) if obj(i).get("/S") == "/GTS_PDFX"]
        require(len(intents) == 1, "output_intent")
        intent = intents[0]
        require(bool(str(intent.get("/OutputConditionIdentifier", "")).strip()), "output_intent_identifier")
        icc = obj(intent.get("/DestOutputProfile"))
        require(isinstance(icc, StreamObject), "embedded_icc")
        profile_bytes = icc.get_data()
        require(len(profile_bytes) >= 132 and int.from_bytes(profile_bytes[:4], "big") == len(profile_bytes)
                and profile_bytes[36:40] == b"acsp", "icc_header")
        require(icc.get("/N") == {b"GRAY": 1, b"RGB ": 3, b"CMYK": 4}.get(profile_bytes[16:20]), "icc_components")
        if icc_sha256:
            require(hashlib.sha256(profile_bytes).hexdigest().lower() == icc_sha256.lower(), "icc_source_bytes")

        pages = list(reader.pages)
        require(0 < len(pages) <= 1000, "page_count")
        page_ids = {ref(page): i for i, page in enumerate(pages)}
        for page in pages:
            boxes = [list(map(float, page[k])) for k in ("/MediaBox", "/CropBox", "/BleedBox", "/TrimBox")]
            require("/ArtBox" not in page and all(len(b) == 4 and b[0] < b[2] and b[1] < b[3] for b in boxes), "page_boxes")
            require(all(a[0] <= b[0] and a[1] <= b[1] and a[2] >= b[2] and a[3] >= b[3]
                        for a, b in zip(boxes, boxes[1:])), "page_boxes")
            for annotation in page.get("/Annots", []):
                a, b = list(map(float, obj(annotation)["/Rect"])), boxes[2]
                require(a[2] < b[0] or a[0] > b[2] or a[3] < b[1] or a[1] > b[3], "annotation_print_area")

        require("/DPartRoot" in root, "hierarchy_root")
        droot_ref = root.raw_get("/DPartRoot")
        droot = obj(droot_ref)
        require(droot.get("/Type") == "/DPartRoot", "hierarchy_root_type")
        require(list(droot.get("/NodeNameList", [])) == ["/Job", "/Record", "/Document"], "hierarchy_names")
        require(droot.get("/RecordLevel") == 1, "hierarchy_record_level")
        require("/DPartRootNode" in droot, "hierarchy_root_node")
        visited, parts, leaves = set(), [], []

        def part(value, parent, depth):
            key = ref(value)
            require(key not in visited and depth <= 2 and len(visited) < 10000, "part_cycle_or_depth")
            visited.add(key)
            node = obj(value)
            require(node.get("/Type") == "/DPart", "part_type")
            require(ref(node.get("/Parent")) == ref(parent), "part_parent")
            metadata = data(node.get("/DPM"))
            private = metadata.get("Fullbleed", {})
            require(isinstance(private.get("ID"), str) and bool(private["ID"].strip()), "dpm_id")
            require(isinstance(private.get("Metadata"), dict), "dpm_metadata")
            item = {"depth": depth, "id": private["ID"], "metadata": private["Metadata"]}
            parts.append(item)
            if depth < 2:
                children = obj(node.get("/DParts", []))
                require(bool(children) and "/Start" not in node and "/End" not in node, "part_children")
                child_ids = []
                for child in children:
                    child_ids.append(part(child, value, depth + 1))
                require(len(set(child_ids)) == len(child_ids), "sibling_ids")
            else:
                require(not node.get("/DParts"), "leaf_children")
                # A single-page leaf may omit End; it then defaults to Start.
                start = page_ids.get(ref(node.get("/Start")))
                end = page_ids.get(ref(node.get("/End", node.get("/Start"))))
                require(start is not None and end is not None and start <= end, "leaf_range")
                expected_start = leaves[-1][1] + 1 if leaves else 0
                require(start == expected_start, "range_partition")
                for page in pages[start:end + 1]:
                    require(ref(page.get("/DPart")) == key, "page_backlink")
                leaves.append((start, end))
                item.update(first_page=start + 1, last_page=end + 1)
            return private["ID"]

        part(droot.raw_get("/DPartRootNode"), droot_ref, 0)
        require(bool(leaves) and leaves[-1][1] == len(pages) - 1, "range_coverage")
        seen, xobjects, fonts = set(), set(), set()

        def resources(value, depth=0):
            require(depth <= 64 and len(seen) <= 100000, "object_graph_bounds")
            if isinstance(value, IndirectObject):
                key = ref(value)
                if key in seen:
                    return
                seen.add(key)
                value = obj(value)
            if isinstance(value, DictionaryObject):
                require(not any(k in value for k in ("/JavaScript", "/JS", "/AA", "/OpenAction", "/AcroForm", "/EmbeddedFiles", "/OPI", "/Ref")), "active_or_external_construct")
                require(value.get("/S") not in ("/JavaScript", "/Launch", "/Movie", "/Sound", "/ImportData", "/SubmitForm"), "active_or_external_construct")
                require(not isinstance(value, StreamObject) or "/F" not in value, "external_stream")
                if value.get("/Type") == "/XObject":
                    xobjects.add(ref(value))
                    require(value.get("/GTS_Scope") == "/File", "resource_scope")
                    encapsulated = value.get("/GTS_Encapsulated")
                    require(isinstance(encapsulated, BooleanObject), "resource_hint_type")
                    if encapsulated.value:
                        require(value.get("/Subtype") == "/Image" and not value.get("/SMask") and not value.get("/Mask")
                                and value.get("/Intent") == "/RelativeColorimetric", "resource_encapsulation")
                if value.get("/Type") == "/Font":
                    subtype = value.get("/Subtype")
                    if subtype == "/Type0":
                        require(bool(value.get("/DescendantFonts")), "font_embedding")
                    elif subtype == "/Type3":
                        require(bool(value.get("/CharProcs")), "font_embedding")
                    else:
                        descriptor = obj(value.get("/FontDescriptor", {}))
                        require(any(isinstance(obj(descriptor.get(k)), StreamObject) for k in ("/FontFile", "/FontFile2", "/FontFile3")), "font_embedding")
                    fonts.add(ref(value))
                for key, child in value.items():
                    if key != "/DPM":
                        resources(child, depth + 1)
            elif isinstance(value, ArrayObject):
                for child in value:
                    resources(child, depth + 1)

        resources(reader.trailer)
        text = [page.extract_text() for page in pages]
        records = [p for p in parts if p["depth"] == 1]
        documents = [p for p in parts if p["depth"] == 2]
        if specimen in ("basic", "multipage"):
            require(len(records) == len(documents) == 1 and len(pages) == (1 if specimen == "basic" else 2), "source_grouping")
            markers = ["Conformance specimen"] if specimen == "basic" else ["page one", "page two"]
        elif specimen == "grouped":
            require(len(pages) == 3 and [p["id"] for p in parts] == ["release-validation", "000001", "statement", "insert", "000002", "statement"], "source_grouping")
            require(parts[0]["metadata"] == {"product": "statements", "copies": 1}
                    and records[0]["metadata"] == {"postal_code": "60601", "duplex": True}
                    and documents[1]["metadata"] == {"media": "insert"}, "source_metadata")
            markers = ["Statement A", "Insert A", "Statement B"]
        elif specimen in ("fixed", "reflow"):
            require(len(records) == len(documents) == 2 and leaves[0] == (0, 0) and leaves[1][0] == 1, "source_grouping")
            require(len(pages) == 2 if specimen == "fixed" else len(pages) > 2, "source_pagination")
            markers = ["Alice"] + ["Bob"] * (len(pages) - 1)
        else:
            raise ContractFailure("unknown_specimen")
        require(all(marker in page_text for marker, page_text in zip(markers, text)), "source_page_assignment")
        if specimen in ("fixed", "reflow"):
            require("Bob" not in text[0] and all("Alice" not in t for t in text[1:]), "source_page_assignment")
        if specimen in ("grouped", "fixed", "reflow"):
            images = [set(ref(v) for v in obj(page["/Resources"].get("/XObject", {})).values()
                          if obj(v).get("/Subtype") == "/Image") for page in pages]
            require(all(len(i) == 1 for i in images) and len(set.union(*images)) == 1, "shared_image_reuse")
        result.update(status="passed", errors=[], page_count=len(pages), parts=parts,
                      xobject_count=len(xobjects), font_count=len(fonts))
    except ContractFailure as error:
        result.update(status="failed", errors=[str(error)])
    except Exception as error:
        result.update(status="error", errors=[f"{type(error).__name__}: {error}"])
    return result


def mutation_controls(source: Path, out_dir: Path, icc_sha256: str | None = None) -> dict:
    from pypdf import PdfWriter
    from pypdf.generic import ArrayObject, ByteStringObject, DictionaryObject, NameObject, NullObject, NumberObject, TextStringObject

    def nodes(writer):
        root = writer.root_object["/DPartRoot"]
        job = root["/DPartRootNode"]
        record = job["/DParts"][0].get_object()
        document = record["/DParts"][0].get_object()
        return root, job, record, document

    def put(target, key, value):
        target[NameObject(key)] = value

    def image(writer):
        return next(v.get_object() for v in writer.pages[0]["/Resources"]["/XObject"].values()
                    if v.get_object().get("/Subtype") == "/Image")

    def alter_date(writer):
        metadata = writer.root_object["/Metadata"]
        before = b'pdfvtid:GTS_PDFVTModDate="2026-09-30'
        content = metadata.get_data()
        if before not in content:
            raise ValueError("date mutation was not applied")
        metadata.set_data(content.replace(before, b'pdfvtid:GTS_PDFVTModDate="2026-09-29', 1))

    def remove_font(writer):
        font = next(iter(writer.pages[0]["/Resources"]["/Font"].values())).get_object()
        if font.get("/Subtype") == "/Type0":
            font = font["/DescendantFonts"][0].get_object()
        descriptor = font["/FontDescriptor"]
        for key in ("/FontFile", "/FontFile2", "/FontFile3"):
            descriptor.pop(key, None)

    def swap_contents(writer):
        first, second = writer.pages[:2]
        a, b = first.raw_get("/Contents"), second.raw_get("/Contents")
        put(first, "/Contents", b)
        put(second, "/Contents", a)

    mutations = [
        ("version", "pdf_version", lambda w: setattr(w, "pdf_header", "%PDF-1.7")),
        ("root", "hierarchy_root", lambda w: w.root_object.pop("/DPartRoot")),
        ("root_node", "hierarchy_root_node", lambda w: nodes(w)[0].pop("/DPartRootNode")),
        ("node_names", "hierarchy_names", lambda w: put(nodes(w)[0], "/NodeNameList", ArrayObject([NameObject("/Document")]))),
        ("record_level", "hierarchy_record_level", lambda w: put(nodes(w)[0], "/RecordLevel", NumberObject(2))),
        ("parent", "part_parent", lambda w: put(nodes(w)[3], "/Parent", nodes(w)[0].indirect_reference)),
        ("cycle", "part_cycle_or_depth", lambda w: nodes(w)[1]["/DParts"].append(nodes(w)[1].indirect_reference)),
        ("overlap", "range_partition", lambda w: put(nodes(w)[2]["/DParts"][1].get_object(), "/Start", w.pages[0].indirect_reference)),
        ("backlink", "page_backlink", lambda w: put(w.pages[0], "/DPart", nodes(w)[2]["/DParts"][1])),
        ("dpm_id", "dpm_id", lambda w: put(nodes(w)[1]["/DPM"]["/Fullbleed"], "/ID", TextStringObject(""))),
        ("dpm_type", "dpm_type", lambda w: put(nodes(w)[1]["/DPM"]["/Fullbleed"]["/Metadata"], "/bad", NullObject())),
        ("scope", "resource_scope", lambda w: put(image(w), "/GTS_Scope", NameObject("/Global"))),
        ("encapsulation", "resource_encapsulation", lambda w: image(w).pop("/Intent")),
        ("vt_date", "vt_date_agreement", alter_date),
        ("title", "title_agreement", lambda w: w.add_metadata({"/Title": "Wrong title"})),
        ("trailer_ids", "trailer_ids", lambda w: setattr(w, "_ID", ArrayObject([ByteStringObject(b"x"), ByteStringObject(b"y")]))),
        ("icc", "icc_components", lambda w: put(w.root_object["/OutputIntents"][0].get_object()["/DestOutputProfile"], "/N", NumberObject(4))),
        ("font", "font_embedding", remove_font),
        ("boxes", "page_boxes", lambda w: put(w.pages[0], "/TrimBox", ArrayObject([NumberObject(v) for v in (0, 0, 9999, 9999)]))),
        ("active_content", "active_or_external_construct", lambda w: put(w.root_object, "/OpenAction", DictionaryObject({NameObject("/S"): NameObject("/JavaScript")}))),
        ("source_assignment", "source_page_assignment", swap_contents),
    ]
    out_dir.mkdir(parents=True, exist_ok=True)
    def clone():
        writer = PdfWriter(clone_from=source)
        writer.pdf_header = "%PDF-1.6"
        return writer

    control = out_dir / "rewrite-control.pdf"
    clone().write(control)
    positive = check_pdf(control, "grouped", icc_sha256)
    results = {}
    for name, expected, mutate in mutations:
        writer = clone()
        mutate(writer)
        path = out_dir / f"{name}.pdf"
        writer.write(path)
        validation = check_pdf(path, "grouped", icc_sha256)
        results[name] = {"status": "passed" if validation["status"] == "failed" and expected in validation["errors"] else "failed",
                         "expected_error": expected, "validation": validation}
    return {"status": "passed" if positive["status"] == "passed" and all(v["status"] == "passed" for v in results.values()) else "failed",
            "rewrite_positive_control": positive, "controls": results}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    try:
        inputs = json.loads(args.suite.read_text("utf-8"))
        specimens = inputs["specimens"]
        if set(specimens) != {"basic", "multipage", "grouped", "fixed", "reflow"}:
            raise ValueError("All five known source specimens are required")
        results = {name: check_pdf(Path(path), name, inputs.get("icc_sha256")) for name, path in specimens.items()}
        controls = mutation_controls(Path(specimens["grouped"]), args.out / "mutations", inputs.get("icc_sha256"))
        report = {"schema": SCHEMA, "scope": "separate_parser_writer_contract", "independent_iso_conformance": False,
                  "status": "passed" if all(v["status"] == "passed" for v in results.values()) and controls["status"] == "passed" else "failed",
                  "specimens": results, "negative_controls": controls}
    except Exception as error:
        report = {"schema": SCHEMA, "status": "error", "error": f"{type(error).__name__}: {error}"}
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": report["status"], "report": str(args.out / "report.json")}))
    return {"passed": 0, "failed": 1, "error": 2}[report["status"]]


if __name__ == "__main__":
    raise SystemExit(main())
