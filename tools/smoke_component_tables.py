# SPDX-License-Identifier: MIT
"""Inspect component-authored tables across PDF and static HTML routes."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib.metadata import version
from importlib.resources import files
from io import BytesIO
import json
from pathlib import Path
import runpy

import fullbleed
from fullbleed.ui.experimental import (
    Column, DataTable, Document, Heading, Image, Node, Page, Paragraph, Renderer,
    Strong, Text, Theme, pt, style,
)
from fullbleed.ui.experimental import bootstrap as bs
from pypdf import PdfReader
import pypdfium2 as pdfium

INSPECT = runpy.run_path(str(Path(__file__).with_name("smoke_table_tagging.py")))["inspect"]
VISIBLE_PAGES = runpy.run_path(str(Path(__file__).with_name("smoke_oversized_table.py")))["visible_pages"]
PROFILES = ("none", "tagged", "pdfua1", "pdfua2")
MODES = ("document", "prepared", "styled-export", "host-styled-export")
CASES = ("plain", "cell-css-bold", "span", "strong", "paragraph", "mixed", "list",
         "figure", "nested-table", "bootstrap-data-table", "bootstrap-table", "split-rows", "oversized-cell",
         "oversized-baseline", "oversized-list")
OVERSIZED = ("oversized-cell", "oversized-baseline", "oversized-list")
FIGURE_ALT = "Blue planting-plan square"
SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18"><rect width="18" height="18" fill="#164b8b"/></svg>'


def element(tag, *children, **attrs):
    return Node(tag, tuple(children), tuple(attrs.items()))


def authored(name):
    def cell(row):
        marker = "CELL Ada"
        if name == "span": return Text(marker, class_name="accent")
        if name == "strong": return Strong(marker)
        if name == "paragraph": return Paragraph(marker)
        if name == "mixed": return Paragraph("Before ", Strong(marker), " after.")
        if name == "list": return element("ul", element("li", marker), element("li", "Second item"))
        if name == "figure": return (Paragraph(marker), Image(src="table-figure.svg", alt=FIGURE_ALT))
        if name == "nested-table":
            return DataTable([marker], columns=[Column("Detail", lambda value: Paragraph(value))], caption="Details")
        if name in {"oversized-cell", "oversized-baseline"}:
            return tuple(Paragraph(f"LINE{i:03} ", Strong(marker), " grows with neighbors.") for i in range(12))
        if name == "oversized-list":
            return element("ul", *(element("li", f"LINE{i:03} ", Strong(marker), " grows with neighbors.")
                                   for i in range(12)))
        if name == "split-rows": return Paragraph(f"ROW{row:02} ", Strong(marker))
        if name.startswith("bootstrap"): return Paragraph("Before ", Strong(marker), " after.")
        return marker

    rows = list(range(32)) if name == "split-rows" else [0]
    columns = (Column("Content", cell), Column("Status", lambda _: "READY"))
    table_class = "records bold-cell" if name == "cell-css-bold" else "records"
    if name == "bootstrap-table":
        table = bs.Table(
            element("caption", "Programs"),
            element("thead", element("tr", element("th", Strong("Content"), scope="col"),
                                      element("th", "Status", scope="col"))),
            element("tbody", element("tr", element("td", cell(0)), element("td", "READY"))),
            bordered=True, small=True, caption_top=True, class_name=table_class,
        )
    elif name == "bootstrap-data-table":
        table = bs.DataTable(rows, columns=columns, caption="Programs", bordered=True,
                             striped=True, small=True, class_name=table_class)
    else:
        table = DataTable(rows, columns=columns, caption="Programs", class_name=table_class,
                          row_break="auto" if name in OVERSIZED else None)
        table = table.with_parts(cell=style(padding=pt(4), background="#edf4f0"),
                                 header=style(padding=pt(4), background="#d4e6dc"))
    height = 160 if name in OVERSIZED else 340
    rules = """p {margin:0} h1 {font-size:14pt;margin:0 0 6pt}
.records {width:100%;table-layout:fixed} .records th,.records td {vertical-align:top}
.bold-cell td {font-weight:700} .accent {color:#164b8b;font-weight:700}
ul {margin:0;padding-left:12pt} img {width:18pt;height:18pt}
caption {text-align:left;font-size:9pt;padding-bottom:3pt}
"""
    if name == "oversized-baseline":
        rules += ".records th,.records td {vertical-align:baseline}\n"
    document = Document(Heading("Program table", level=1), table,
                        title="Component table: " + name, lang="en-US",
                        page=Page((pt(320), pt(height)), margin=pt(18)),
                        theme=Theme(text=style(font_family="Inter", font_size=pt(9), line_height=1.3)),
                        styles=rules)
    host = f"""@page {{size:320pt {height}pt;margin:18pt}}
* {{box-sizing:border-box}} html,body {{margin:0;padding:0;font-family:Inter;font-size:9pt;line-height:1.3}}
table {{border-collapse:collapse}} th,td {{padding:4pt;border:0.5pt solid #164b8b}}
th {{background:#dbe8f8;text-align:left}} td {{background:#f2f6fc}}
tr {{break-inside:avoid}} thead {{display:table-header-group}}
""" + rules
    markers = {"CELL Ada": 32 if name == "split-rows" else 12 if name in OVERSIZED else 1,
               "READY": 32 if name == "split-rows" else 1}
    if name == "split-rows": markers.update({f"ROW{i:02}": 1 for i in range(32)})
    if name in OVERSIZED: markers.update({f"LINE{i:03}": 1 for i in range(12)})
    return document, host, markers


class AssetRenderer(Renderer):
    def __init__(self, asset, **options):
        super().__init__(**options)
        self.asset = asset

    def engine_for(self, document):
        engine = super().engine_for(document)
        bundle = fullbleed.AssetBundle()
        bundle.add_file(str(self.asset), "svg", name="table-figure.svg")
        engine.register_bundle(bundle)
        return engine


def check(out, names=CASES, profiles=PROFILES):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=False)
    asset = out / "table-figure.svg"
    asset.write_text(SVG, encoding="utf-8")
    font = Path(str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")))
    records = []
    for profile in profiles:
        renderer = AssetRenderer(asset, font_files=[str(font)], pdf_profile=profile,
                                 document_timestamp="2026-10-18T00:00:00Z")
        for name in names:
            folder = out / profile / name
            folder.mkdir(parents=True)
            document, host, markers = authored(name)
            source = document.compile(a11y_mode="raise")
            prepared = document.prepare(renderer, a11y_mode="raise")
            styled_html = document.to_html(include_styles=True, a11y_mode="raise")
            plain_html = document.to_html(include_styles=False, a11y_mode="raise")
            assert "<style" in styled_html and "<style" not in plain_html and " style=" not in plain_html
            assert all("<script" not in html.lower() for html in (styled_html, plain_html))
            assert "records" in plain_html
            for filename, content in (("source.html", source.html), ("source.css", source.css),
                                      ("styled.html", styled_html), ("unstyled.html", plain_html), ("host.css", host)):
                (folder / filename).write_text(content, encoding="utf-8", newline="\n")
            (folder / "source-a11y.json").write_text(json.dumps(source.a11y_report, indent=2)+"\n", encoding="utf-8")
            engine = renderer.engine_for(document)
            outputs, checks = {}, {}
            for mode in MODES:
                def render():
                    if mode == "document": return bytes(document.render_pdf(renderer, a11y_mode="raise"))
                    if mode == "prepared": return bytes(prepared.render_pdf())
                    return bytes(engine.render_pdf(styled_html if mode == "styled-export" else plain_html,
                                                   "" if mode == "styled-export" else host))
                data = render()
                (folder / (mode+".pdf")).write_bytes(data)
                outputs[mode] = data
                reader = PdfReader(BytesIO(data))
                if profile == "none":
                    result = {"errors": [], "pages": [{"text": page.extract_text()} for page in reader.pages]}
                else:
                    result = INSPECT(data)
                result["page_content_sha256"] = [
                    sha256(page.get_contents().get_data()).hexdigest() for page in reader.pages]
                result["page_boxes"] = [list(page.mediabox) for page in reader.pages]
                errors = result["errors"]
                if data != render(): errors.append("Repeated rendering changed PDF bytes")
                text = " ".join(" ".join(p["text"] for p in result["pages"]).split())
                counts = {marker: text.count(marker) for marker in markers}
                if counts != markers: errors.append("Missing or duplicate cell text")
                if name in (*OVERSIZED, "split-rows") and len(result["pages"]) < 2:
                    errors.append("Pagination control did not cross a page")
                preview = folder/(mode+"-visible")
                save_images = profile == "pdfua1" and name in OVERSIZED
                if save_images: preview.mkdir()
                visible = VISIBLE_PAGES(data, preview, save_images)
                result["visible_pages"] = visible
                if any(page["outside_glyphs"] for page in visible):
                    errors.append("Visible text extends beyond the authored page margins")
                if any(page["glyphs_without_visible_ink"] for page in visible):
                    errors.append("Extracted text has no visible ink")
                body_marker = "LINE" if name in OVERSIZED else "ROW" if name == "split-rows" else None
                if body_marker and any(body_marker not in page["text"] for page in visible):
                    errors.append("A continuation page contains no body content")
                if profile != "none":
                    if result["roles"].get("/TH", 0) < 2: errors.append("Scoped column headers lost")
                    if any(c["role"] == "/TH" and c["scope"] != "/Column" for c in result["cells"]):
                        errors.append("Column scope lost")
                    if name == "figure" and result["figures"] != [FIGURE_ALT]: errors.append("Figure alternative lost")
                if profile == "pdfua1" and mode in {"document", "host-styled-export"}:
                    images = []
                    with pdfium.PdfDocument(data) as pdf:
                        for i in range(len(pdf)):
                            page = pdf[i]; bitmap = page.render(scale=2); image = bitmap.to_pil()
                            image.save(folder / f"{mode}-page-{i+1}.png")
                            images.append({"size": list(image.size), "rgba_sha256": sha256(image.convert("RGBA").tobytes()).hexdigest()})
                            image.close(); bitmap.close(); page.close()
                    result["pdfium"] = images
                result.update(profile=profile, name=name, mode=mode, ok=not errors,
                              pdf_sha256=sha256(data).hexdigest(), marker_counts=counts)
                records.append(result)
                checks[mode] = result
            if outputs["document"] != outputs["prepared"]:
                checks["prepared"]["errors"].append("Prepared document changed PDF bytes")
            # Embedding CSS adds HTML nodes and changes generated structure IDs.
            # Require identical page content and relationships, not those IDs.
            for key in ("page_content_sha256", "page_boxes", "roles", "figures"):
                if checks["document"].get(key) != checks["styled-export"].get(key):
                    checks["styled-export"]["errors"].append("Styled HTML export changed " + key)
            def relationships(result):
                return [(c["role"], c["scope"], c["columns"], c["rows"], len(c["headers"]))
                        for c in result.get("cells", [])]
            if relationships(checks["document"]) != relationships(checks["styled-export"]):
                checks["styled-export"]["errors"].append("Styled HTML export changed table relationships")
            if [p["rgba_sha256"] for p in checks["document"]["visible_pages"]] != [
                    p["rgba_sha256"] for p in checks["styled-export"]["visible_pages"]]:
                checks["styled-export"]["errors"].append("Styled HTML export changed visible page pixels")
            for result in checks.values():
                result["ok"] = not result["errors"]
            assert document.compile(a11y_mode="raise") == source, "Authoring source mutated"
    report = {"schema": "fullbleed.component_tables.v1", "version": version("fullbleed"),
              "ok": all(c["ok"] for c in records), "cases": records,
              "font_sha256": sha256(font.read_bytes()).hexdigest(), "asset_sha256": sha256(asset.read_bytes()).hexdigest(),
              "scope": "Authored table content ownership, visible ink/bounds, static HTML export and pagination regression; not general PDF/UA conformance."}
    (out / "verification.json").write_text(json.dumps(report, indent=2)+"\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--case", action="append", choices=CASES)
    parser.add_argument("--profile", action="append", choices=PROFILES)
    args = parser.parse_args()
    report = check(args.out, args.case or CASES, args.profile or PROFILES)
    print(json.dumps({"ok": report["ok"], "cases": len(report["cases"]),
                      "failed": sum(not c["ok"] for c in report["cases"])}))
    raise SystemExit(0 if report["ok"] else 1)
