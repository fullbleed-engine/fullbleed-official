"""Content/layout qualification using pypdf and PDFium, not any rendering adapter."""
from __future__ import annotations

import hashlib
import re
from pathlib import Path

import pypdfium2 as pdfium
from pypdf import PdfReader, PdfWriter, Transformation


def compact(text: str) -> str:
    return re.sub(r"\s+", "", text)


def resolved(value):
    return value.get_object() if hasattr(value, "get_object") else value


def check_pdf(path: Path, contract: dict, token: str, previews: Path | None = None) -> dict:
    failures = []
    reader = PdfReader(path)
    count = len(reader.pages)
    if not contract["min_pages"] <= count <= contract["max_pages"]:
        failures.append(f"page count {count} outside declared range")
    texts = [compact(p.extract_text()) for p in reader.pages]
    joined = "".join(texts)
    for phrase in [token, *contract["markers"]]:
        occurrences = joined.count(compact(phrase))
        if occurrences != 1:
            failures.append(f"{phrase}: expected once, found {occurrences}")
    positions = [joined.find(compact(s)) for s in contract["markers"]]
    if positions != sorted(positions):
        failures.append("record order differs from input")
    for phrase in contract["required"]:
        if compact(phrase) not in joined:
            failures.append(f"required text missing: {phrase}")
    for phrase in contract["text_segments"]:
        expected = compact(phrase.replace(contract["token"], token))
        if expected not in joined:
            failures.append(f"body text segment missing: {expected[:100]}")
    for row in contract["row_text"]:
        if compact("".join(row)) not in joined:
            failures.append(f"row values differ or are separated: {row[0]}")
    for i, marker in enumerate(contract.get("page_markers", [])):
        if i >= count or marker not in texts[i]:
            failures.append(f"{marker} missing from page {i + 1}")
    heading = contract.get("repeated_heading")
    if heading:
        for i, text in enumerate(texts):
            if any(m in text for m in contract["markers"]) and compact(heading) not in text:
                failures.append(f"table heading missing on page {i + 1}")
    font_names = set()
    font_objects = set()

    def inspect_resources(resources):
        resources = resources.get_object()
        for reference in resolved(resources.get("/Font", {})).values():
            font = reference.get_object()
            identity = (getattr(reference, "idnum", None), str(font.get("/BaseFont")))
            if identity in font_objects:
                continue
            font_objects.add(identity)
            name = str(font.get("/BaseFont", ""))
            font_names.add(name)
            descendants = font.get("/DescendantFonts", [font])
            for descendant in descendants:
                item = descendant.get_object()
                desc = item.get("/FontDescriptor")
                if not desc or not any(key in desc.get_object() for key in ("/FontFile", "/FontFile2", "/FontFile3")):
                    failures.append(f"font is not embedded: {name}")
            if contract["font_name_contains"].lower() not in name.lower():
                failures.append(f"unexpected font: {name}")
        for reference in resolved(resources.get("/XObject", {})).values():
            obj = reference.get_object()
            if obj.get("/Subtype") == "/Form" and "/Resources" in obj:
                inspect_resources(obj["/Resources"])

    for i, page in enumerate(reader.pages):
        actual = [float(page.mediabox.width), float(page.mediabox.height)]
        if any(abs(a - b) > 0.8 for a, b in zip(actual, contract["page_size_pt"])):
            failures.append(f"page {i + 1} size differs: {actual}")
        inspect_resources(page["/Resources"])
    if not font_names:
        failures.append("no fonts found")

    character_count = 0
    outside_count = 0
    page_details = []
    with pdfium.PdfDocument(path) as pdf:
        for i in range(len(pdf)):
            page = pdf[i]
            width, height = page.get_size()
            textpage = page.get_textpage()
            boxes = []
            for char in range(textpage.count_chars()):
                value = textpage.get_text_range(char, 1)
                if not value.strip():
                    continue
                character_count += 1
                left, bottom, right, top = textpage.get_charbox(char)
                inset = contract["text_page_inset_pt"]
                if left < inset or bottom < inset or right > width - inset or top > height - inset:
                    outside_count += 1
                    if len(boxes) < 8:
                        boxes.append({"text": value, "box": [left, bottom, right, top]})
            page_details.append({"page": i + 1, "width_pt": width, "height_pt": height,
                                 "outside_examples": boxes})
            if previews:
                previews.mkdir(parents=True, exist_ok=True)
                bitmap = page.render(scale=1.25)
                bitmap.to_pil().save(previews / f"page-{i + 1}.png")
                bitmap.close()
            textpage.close()
            page.close()
    if outside_count:
        failures.append(f"{outside_count} visible character boxes outside declared inset")
    return {"passed": not failures, "failures": failures, "pages": count,
            "fonts": sorted(font_names), "visible_characters": character_count,
            "page_details": page_details, "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def negative_controls(path: Path, contract: dict, token: str, directory: Path) -> dict:
    """Prove qualification rejects wrong records, lost pages, and missing fonts."""
    directory.mkdir(parents=True, exist_ok=True)
    wrong_token = check_pdf(path, contract, "RUN-999999")
    writer = PdfWriter()
    reader = PdfReader(path)
    for page in reader.pages[:-1]:
        writer.add_page(page)
    missing = directory / "missing-last-page.pdf"
    writer.write(missing)
    missing_page = check_pdf(missing, contract, token)
    writer = PdfWriter(clone_from=path)
    for page in writer.pages:
        for reference in resolved(page["/Resources"].get("/Font", {})).values():
            font = reference.get_object()
            for reference in font.get("/DescendantFonts", [font]):
                descendant = reference.get_object()
                if "/FontDescriptor" in descendant:
                    desc = descendant["/FontDescriptor"].get_object()
                    for key in ("/FontFile", "/FontFile2", "/FontFile3"):
                        desc.pop(key, None)
    unembedded = directory / "unembedded-font.pdf"
    writer.write(unembedded)
    missing_font = check_pdf(unembedded, contract, token)
    writer = PdfWriter(clone_from=path)
    writer.pages[0].add_transformation(Transformation().translate(tx=-60, ty=0))
    shifted = directory / "outside-page-inset.pdf"
    writer.write(shifted)
    outside = check_pdf(shifted, contract, token)
    return {"passed": all(not x["passed"] for x in [wrong_token, missing_page, missing_font, outside]),
            "wrong_token": wrong_token["failures"], "missing_page": missing_page["failures"],
            "missing_font": missing_font["failures"], "outside_page_inset": outside["failures"]}
