#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Inspect the actual PDF's structure and optionally run veraPDF's UA-1 profile."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import subprocess

from pypdf import PdfReader
from guide import IMAGE_ALT, TITLE


def normalize(text):
    return " ".join(text.split())


def inspect_structure(pdf):
    reader = PdfReader(pdf)
    catalog = reader.trailer["/Root"]
    assert len(reader.pages) == 2
    assert reader.metadata.title == TITLE
    assert catalog["/Lang"] == "en-US"
    assert catalog["/MarkInfo"]["/Marked"]
    assert catalog["/ViewerPreferences"]["/DisplayDocTitle"]
    chunks = defaultdict(list)
    page_numbers = {}
    artifact_text = []
    for page_number, page in enumerate(reader.pages, 1):
        page_numbers[page.indirect_reference.idnum] = page_number
        stack = []

        def before(op, args, cm, tm):
            if op == b"BDC":
                props = args[1].get_object()
                stack.append((str(args[0]), int(props["/MCID"]) if "/MCID" in props else None))
            elif op == b"BMC":
                stack.append((str(args[0]), None))
            elif op == b"EMC":
                stack.pop()

        def text(value, cm, tm, font, size):
            if any(role == "/Artifact" for role, _ in stack):
                artifact_text.append(value)
            else:
                for _, mcid in reversed(stack):
                    if mcid is not None:
                        chunks[(page_number, mcid)].append(value)
                        break

        page.extract_text(visitor_operand_before=before, visitor_text=text)
    roles = Counter()
    nodes = []
    leaves = []

    def walk(value, page_number=None):
        value = value.get_object() if hasattr(value, "get_object") else value
        if isinstance(value, list):
            return " ".join(walk(item, page_number) for item in value)
        if isinstance(value, int):
            text = normalize("".join(chunks[(page_number, value)]))
            leaves.append(dict(page=page_number, mcid=value, text=text))
            return text
        if not isinstance(value, dict):
            return ""
        if "/Pg" in value:
            page_number = page_numbers[value.raw_get("/Pg").idnum]
        role = str(value.get("/S", ""))
        roles[role] += 1
        node = dict(role=role, page=page_number)
        nodes.append(node)
        node["text"] = normalize(walk(value.get("/K", []), page_number))
        if "/Alt" in value:
            node["alt"] = str(value["/Alt"])
        if "/A" in value:
            node["attributes"] = {str(k): str(v) for k, v in value["/A"].items()}
        return node["text"]

    walk(catalog["/StructTreeRoot"]["/K"])
    headings = [node for node in nodes if node["role"] in {"/H1", "/H2", "/H3"}]
    expected = [
        ("/H1", "A place to grow. A place to belong."),
        ("/H2", "Find your starting point"),
        ("/H3", "Grow something"), ("/H3", "Share a table"), ("/H3", "Make it last"),
        ("/H2", "Every visit starts with hello"),
        ("/H2", "Make room for a good day."), ("/H2", "Plan a comfortable visit"),
        ("/H3", "An easy arrival"), ("/H3", "Choose your pace"),
        ("/H2", "A small first step is enough"), ("/H2", "About this guide"),
    ]
    assert [(node["role"], node["text"]) for node in headings] == expected, headings
    assert roles["/Figure"] == 1
    assert next(node["alt"] for node in nodes if node["role"] == "/Figure") == IMAGE_ALT
    assert roles["/Table"] == 1 and roles["/TH"] == 3 and roles["/TD"] == 9
    headers = [node for node in nodes if node["role"] == "/TH"]
    assert [node["text"] for node in headers] == ["Program", "When", "What to expect"]
    assert all(node["attributes"]["/Scope"] == "/Column" for node in headers)
    assert roles["/L"] == 2 and roles["/LI"] == 6
    reading = normalize(" ".join(leaf["text"] for leaf in leaves))
    assert "Community field guide · 1" not in reading and "Community field guide · 2" not in reading
    assert not any(leaf["text"] in {"01", "02", "03"} for leaf in leaves)
    markers = ["Garden together", "The shared table", "Make & mend", "An easy arrival",
               "Enter through the wide gate", "A welcome volunteer", "Choose your pace",
               "Take part seated", "Ask for written instructions", "A small first step",
               "About this guide", "Fullbleed documentation"]
    positions = [reading.index(marker) for marker in markers]
    assert positions == sorted(positions), "The logical reading order changed"
    return dict(ok=True, title=reader.metadata.title, language=catalog["/Lang"],
                page_count=len(reader.pages), roles=dict(roles), headings=headings,
                nodes=nodes, reading_order=leaves, decorative_text_omitted=True,
                pdf_link_annotations=sum(len(page.get("/Annots", [])) for page in reader.pages),
                scope="Actual PDF structure, MCID text and semantic order; not an assistive-technology user test.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path(__file__).parent / "output")
    parser.add_argument("--verapdf-cp", type=Path,
                        help="Classpath containing org.verapdf.apps.GreenfieldCliWrapper")
    args = parser.parse_args()
    out = args.out.resolve()
    pdf = out / "common-ground-guide.pdf"
    report = inspect_structure(pdf)
    report["pdf_sha256"] = hashlib.sha256(pdf.read_bytes()).hexdigest()
    (out / "structure.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    summary = dict(ok=True, pdf_sha256=report["pdf_sha256"], structure_passed=True,
                   external_validation="not run")
    if args.verapdf_cp:
        command = ["java", "-cp", str(args.verapdf_cp), "org.verapdf.apps.GreenfieldCliWrapper"]
        result = subprocess.run(command + ["--format", "json", "--maxfailuresdisplayed", "-1",
                                           "-f", "ua1", str(pdf)], capture_output=True)
        (out / "pdfua1.verapdf.json").write_bytes(result.stdout)
        (out / "pdfua1.verapdf.stderr.txt").write_bytes(result.stderr)
        parsed = json.loads(result.stdout)
        validated = parsed["report"]["jobs"][0]["validationResult"][0]
        assert result.returncode == 0 and validated["compliant"], validated
        summary["external_validation"] = dict(profile=validated["profileName"],
                                              compliant=validated["compliant"],
                                              details=validated["details"],
                                              build=parsed["report"]["buildInformation"])
    (out / "validation.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
