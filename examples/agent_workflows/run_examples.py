#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run compact, complete agent-oriented Fullbleed workflows."""

from __future__ import annotations

import argparse
from html import escape
from importlib import metadata, resources
import json
from pathlib import Path
import sys
from typing import Any

import fullbleed


DATA_DIR = Path(__file__).resolve().parent / "data"
BASE_CSS = """
@page { size: letter; margin: 0.55in; }
body { margin: 0; font-family: Inter, Helvetica, sans-serif; color: #172033;
       font-size: 10pt; line-height: 1.35; }
h1 { margin: 0 0 14pt; color: #173f71; font-size: 23pt; }
h2 { margin: 14pt 0 6pt; color: #285987; font-size: 14pt; }
table { width: 100%; border-collapse: collapse; }
th, td { border-bottom: 0.6pt solid #ccd5df; padding: 6pt; text-align: left; }
th { color: #285987; }
.meta { color: #59677a; }
.total { font-size: 15pt; font-weight: 700; text-align: right; margin-top: 16pt; }
"""


def _font_path() -> str:
    return str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))


def _text(path: Path) -> str:
    report = fullbleed.extract_pdf_page_texts(str(path))
    return "\n".join(page.get("text") or "" for page in report.get("pages", []))


def _validate(path: Path, markers: list[str], *, min_pages: int = 1) -> dict[str, Any]:
    inspection = dict(fullbleed.inspect_pdf(str(path)))
    text = _text(path)
    failures = []
    if int(inspection.get("page_count") or 0) < min_pages:
        failures.append(f"expected at least {min_pages} page(s)")
    for marker in markers:
        if marker not in text:
            failures.append(f"missing text marker {marker!r}")
    return {
        "ok": not failures,
        "path": str(path),
        "bytes": path.stat().st_size,
        "page_count": inspection.get("page_count"),
        "profile": inspection.get("profile", {}),
        "composition": inspection.get("composition", {}),
        "failures": failures,
    }


def _ordinary(
    out: Path,
    name: str,
    html: str,
    css: str,
    markers: list[str],
    *,
    min_pages: int = 1,
    engine_options: dict[str, Any] | None = None,
) -> dict[str, Any]:
    case_dir = out / name
    case_dir.mkdir(parents=True, exist_ok=True)
    source_html = case_dir / "document.html"
    source_css = case_dir / "document.css"
    pdf_path = case_dir / "document.pdf"
    preview_dir = case_dir / "preview"
    source_html.write_text(html, encoding="utf-8")
    source_css.write_text(css, encoding="utf-8")
    options = {"font_files": [_font_path()], **(engine_options or {})}
    engine = fullbleed.PdfEngine(**options)
    written = engine.render_pdf_to_file(html, css, str(pdf_path))
    previews = list(
        engine.render_finalized_pdf_image_pages_to_dir(
            str(pdf_path), str(preview_dir), 144, name
        )
    )
    result = _validate(pdf_path, markers, min_pages=min_pages)
    result.update(
        {
            "id": name,
            "bytes_written": int(written),
            "sources": [str(source_html), str(source_css)],
            "preview_paths": previews,
        }
    )
    return result


def invoice(out: Path) -> dict[str, Any]:
    data = json.loads((DATA_DIR / "invoice.json").read_text(encoding="utf-8"))
    rows = "".join(
        f"<tr><td>{escape(item['label'])}</td><td>{item['quantity']}</td>"
        f"<td>USD {item['amount']}</td></tr>"
        for item in data["items"]
    )
    html = (
        "<!doctype html><html lang='en'><head><title>Invoice</title></head><body>"
        f"<main><h1>Invoice {escape(data['invoice'])}</h1>"
        f"<p class='meta'>Bill to: {escape(data['customer'])}</p>"
        "<table><thead><tr><th>Item</th><th>Qty</th><th>Amount</th></tr></thead>"
        f"<tbody>{rows}</tbody></table><p class='total'>{escape(data['total'])}</p>"
        "</main></body></html>"
    )
    return _ordinary(
        out,
        "invoice",
        html,
        BASE_CSS,
        [data["invoice"], data["customer"], "1,284.50"],
        engine_options={"document_lang": "en-US", "document_title": "Invoice"},
    )


def business_report(out: Path) -> dict[str, Any]:
    data = json.loads((DATA_DIR / "report.json").read_text(encoding="utf-8"))
    sections = [
        {
            "heading": f"Operating segment {index:02d}",
            "text": (
                f"{data['report_id']} segment {index:02d} recorded deterministic document activity. "
                "The review covers service quality, delivery controls, and the next measured action. "
            )
            * 3,
        }
        for index in range(1, int(data["segment_count"]) + 1)
    ]
    body = "".join(
        f"<section><h2>{escape(item['heading'])}</h2><p>{escape(item['text'])}</p></section>"
        for item in sections
    )
    html = (
        "<!doctype html><html lang='en'><head><title>Quarterly report</title></head>"
        f"<body><main><h1>{escape(data['title'])}</h1>"
        f"<p class='meta'>{escape(data['report_id'])} &middot; Executive Summary</p>"
        f"{body}<h2>Appendix Alpha</h2><p>End of reviewed dataset.</p>"
        "</main></body></html>"
    )
    return _ordinary(
        out,
        "business_report",
        html,
        BASE_CSS + "section { break-inside: avoid; }",
        [data["report_id"], "Executive Summary", "Appendix Alpha"],
        min_pages=3,
        engine_options={
            "document_lang": "en-US",
            "document_title": data["title"],
        },
    )


def accessible(out: Path) -> dict[str, Any]:
    html = (
        "<!doctype html><html lang='en-US'><head><title>Accessible service notice</title>"
        "</head><body><main><h1>Accessible Service Notice</h1>"
        "<p>ACCESS-2026-3001</p><h2>Purpose</h2>"
        "<p>This notice provides a clear reading order and meaningful document structure.</p>"
        "<h2>Contact</h2><p>Email support@example.invalid for an alternative format.</p>"
        "</main></body></html>"
    )
    result = _ordinary(
        out,
        "accessible",
        html,
        BASE_CSS,
        ["ACCESS-2026-3001", "Accessible Service Notice"],
        engine_options={
            "pdf_profile": "pdfua1",
            "document_lang": "en-US",
            "document_title": "Accessible Service Notice",
        },
    )
    profile = result.get("profile") or {}
    required = ["struct_tree_root_present", "mark_info_present", "lang_present"]
    if not any(claim in {"pdfua1", "pdfua2"} for claim in profile.get("claims", [])):
        result["failures"].append("missing PDF/UA profile claim")
    for key in required:
        if not profile.get(key):
            result["failures"].append(f"profile.{key} is not true")
    if profile.get("seed_blockers"):
        result["failures"].append("PDF/UA seed blockers are present")
    result["ok"] = not result["failures"]
    return result


def compiled_vdp(out: Path) -> dict[str, Any]:
    case_dir = out / "compiled_vdp"
    case_dir.mkdir(parents=True, exist_ok=True)
    output = case_dir / "statements.pdf"
    template = (
        "<main><h1>Account Statement</h1><p>Statement: {{statement_id}}</p>"
        "<p>Customer: {{customer}}</p><p>Balance: {{balance}}</p></main>"
    )
    css = "@page{size:letter;margin:.6in}body{font:11pt Helvetica,sans-serif}"
    bindings = {
        "statement_id": [f"VDP-{index:04d}" for index in range(1, 101)],
        "customer": [f"Customer {index:04d}" for index in range(1, 101)],
        "balance": [f"USD {index * 17:,}.00" for index in range(1, 101)],
    }
    compiled = fullbleed.PdfEngine().compile_pdf(template, css)
    written = compiled.render_pdf_bindings_to_file(bindings, str(output))
    result = _validate(output, ["VDP-0001", "VDP-0100"], min_pages=100)
    result.update(
        {
            "id": "compiled_vdp",
            "bytes_written": int(written),
            "record_count": 100,
            "compiled_stats": compiled.stats(),
            "preview_paths": [],
        }
    )
    return result


def compiled_reflow(out: Path) -> dict[str, Any]:
    case_dir = out / "compiled_reflow"
    case_dir.mkdir(parents=True, exist_ok=True)
    output = case_dir / "reflow.pdf"
    template = (
        "<main><h1>{{record_id}}</h1><div class='content'>{{content}}</div>"
        "<p>END {{record_id}}</p></main>"
    )
    css = (
        "@page{size:260pt 190pt;margin:18pt}body{font:9pt/11pt Helvetica,sans-serif}"
        "h1{font-size:14pt}.content{white-space:pre-wrap}"
    )
    bindings = {
        "record_id": [f"FLOW-{index:03d}" for index in range(1, 21)],
        "content": [
            "\n".join(f"record {index:03d} line {line:03d}" for line in range(index * 2))
            for index in range(1, 21)
        ],
    }
    compiled = fullbleed.PdfEngine().compile_pdf(template, css)
    written = compiled.render_pdf_reflow_bindings_to_file(
        bindings,
        str(output),
        compression="throughput",
    )
    result = _validate(output, ["FLOW-001", "FLOW-020", "END FLOW-020"], min_pages=20)
    result.update(
        {
            "id": "compiled_reflow",
            "bytes_written": int(written),
            "record_count": 20,
            "compiled_stats": compiled.stats(),
            "preview_paths": [],
        }
    )
    return result


def run_all(out: Path) -> dict[str, Any]:
    out.mkdir(parents=True, exist_ok=True)
    cases = [
        invoice(out),
        business_report(out),
        accessible(out),
        compiled_vdp(out),
        compiled_reflow(out),
    ]
    return {
        "schema": "fullbleed.agent_examples_result.v1",
        "ok": all(case["ok"] for case in cases),
        "fullbleed_version": metadata.version("fullbleed"),
        "output_root": str(out.resolve()),
        "cases": cases,
        "metrics": {
            "total": len(cases),
            "passed": sum(1 for case in cases if case["ok"]),
            "failed": sum(1 for case in cases if not case["ok"]),
            "pages": sum(int(case.get("page_count") or 0) for case in cases),
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path("output/agent-examples"))
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)
    result = run_all(arguments.out)
    if arguments.json:
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(
            f"[{'ok' if result['ok'] else 'fail'}] agent examples: "
            f"{result['metrics']['passed']}/{result['metrics']['total']} passed\n"
        )
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
