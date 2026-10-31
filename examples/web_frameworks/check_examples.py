#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise all three framework routes and inspect their delivered PDF bytes."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata
import json
import os
from pathlib import Path
import shutil

import fullbleed
from pypdf import PdfReader

from demo import preview_path
from check_layout import check_layouts, inspect_invoice
from invoice import load_invoice, render_invoice


def inspect(path: Path, markers: list[str], expected_pages: int = 1) -> dict:
    report = dict(fullbleed.inspect_pdf(str(path)))
    text = "\n".join(
        page.get("text", "") for page in fullbleed.extract_pdf_page_texts(str(path))["pages"]
    )
    text = " ".join(text.split())
    for marker in markers:
        assert marker in text, (path.name, "missing PDF text", marker)
    assert report["page_count"] == expected_pages, (path.name, "unexpected page count")
    assert report["profile"]["embedded_font_count"] >= 4, (path.name, "design fonts not embedded")
    assert not report["warnings"], (path.name, report["warnings"])
    assert not report["composition"]["issues"], (path.name, report["composition"])
    independent = PdfReader(path)
    assert len(independent.pages) == expected_pages
    independent_text = "".join("".join(page.extract_text().split()) for page in independent.pages)
    for marker in markers:
        assert "".join(marker.split()) in independent_text, (path.name, "independent text", marker)
    if "Design workshop" in markers:
        assert independent_text.count("Designworkshop") == 1, (path.name, "duplicate text")
    return report


def check_client(name: str, client, out: Path, invoice_id: str = "INV-1042") -> dict:
    response = client.get(f"/invoices/{invoice_id}.pdf")
    assert response.status_code == 200, (name, response.status_code)
    assert response.headers["Content-Type"] == "application/pdf", name
    assert response.headers["Content-Disposition"] == 'attachment; filename="invoice.pdf"', name
    assert response.headers["Cache-Control"] == "private, no-store", name
    content = response.data if name == "flask" else response.content
    suffix = "" if invoice_id == "INV-1042" else "-long"
    pdf_path = out / f"{name}{suffix}.pdf"
    pdf_path.write_bytes(content)
    data = load_invoice(invoice_id)
    layout = inspect_invoice(pdf_path, data)
    expected_pages = 1 if invoice_id == "INV-1042" else 3
    report = inspect(pdf_path, [invoice_id, data["customer"], f"USD {layout['total']}"], expected_pages)
    repeat = client.get(f"/invoices/{invoice_id}.pdf")
    assert content == (repeat.data if name == "flask" else repeat.content), name
    assert client.get("/invoices/DOES-NOT-EXIST.pdf").status_code == 404, name
    return {
        "framework": name,
        "invoice_id": invoice_id,
        "ok": True,
        "http_status": response.status_code,
        "missing_invoice_status": 404,
        "content_type": response.headers["Content-Type"],
        "content_disposition": response.headers["Content-Disposition"],
        "page_count": report["page_count"],
        "embedded_font_count": report["profile"]["embedded_font_count"],
        "bytes": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
        "repeat_request_identical": True,
        "pdf": str(pdf_path),
        "layout": layout,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/web-framework-check"))
    parser.add_argument("--update-preview", action="store_true", help="Refresh the local demo's saved preview after editing the invoice.")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    fonts = Path(__file__).resolve().parent / "fonts"
    for item in json.loads((fonts / "sources.json").read_text(encoding="utf-8"))["files"]:
        font_bytes = (fonts / item["file"]).read_bytes()
        assert len(font_bytes) == item["bytes"]
        assert hashlib.sha256(font_bytes).hexdigest() == item["sha256"], item["file"]

    from fastapi.testclient import TestClient
    from fastapi_app import app as fastapi_app
    from flask_app import app as flask_app

    with TestClient(fastapi_app) as client:
        results = [check_client("fastapi", client, out, number) for number in ["INV-1042", "INV-1043"]]
        schema = client.get("/openapi.json").json()
        assert "application/pdf" in schema["paths"]["/invoices/{invoice_id}.pdf"]["get"]["responses"]["200"]["content"]
    with flask_app.test_client() as client:
        results.extend(check_client("flask", client, out, number) for number in ["INV-1042", "INV-1043"])

    os.environ.setdefault("DJANGO_SETTINGS_MODULE", "django_app")
    import django
    from django.test import Client

    django.setup()
    results.extend(check_client("django", Client(), out, number) for number in ["INV-1042", "INV-1043"])
    for number in ["INV-1042", "INV-1043"]:
        assert len({result["sha256"] for result in results if result["invoice_id"] == number}) == 1, "Frameworks changed the PDF bytes"

    # Exercise literal markup-like customer text independently of the fixed HTTP sample.
    data = load_invoice("INV-1042")
    assert data is not None
    data["customer"] = "North <East> & ${rows}"
    escaped = out / "escaped-text.pdf"
    escaped.write_bytes(render_invoice(data))
    inspect(escaped, [data["customer"], "USD 1,870.00"])
    inspect_invoice(escaped, data)
    layouts = check_layouts(out / "layout")

    previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages_to_dir(
        str(out / "fastapi.pdf"), str(out / "preview"), 110, "invoice"
    )
    assert len(previews) == 1 and Path(previews[0]).read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    saved_preview = preview_path()
    if args.update_preview:
        shutil.copyfile(previews[0], saved_preview)
    assert Path(previews[0]).read_bytes() == saved_preview.read_bytes(), "Saved preview is stale; rerun with --update-preview."
    from check_http import check_servers
    http = check_servers(out, (out / "fastapi.pdf").read_bytes(), (out / "fastapi-long.pdf").read_bytes())
    report = {
        "ok": True,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "versions": {name: metadata.version(name) for name in ["fullbleed", "fastapi", "Flask", "Django", "httpx2", "pypdf", "pypdfium2"]},
        "frameworks": results,
        "http_servers": http,
        "all_framework_pdf_bytes_identical": True,
        "literal_customer_text_check": "passed",
        "font_and_license_source_hashes": "passed",
        "invoice_layouts": layouts,
        "fastapi_openapi_pdf_type": "passed",
        "preview": str(previews[0]),
        "saved_preview": saved_preview.name,
        "preview_sha256": hashlib.sha256(Path(previews[0]).read_bytes()).hexdigest(),
        "scope": "Test clients and actual local HTTP servers, independent PDF text, headers, font embedding, saved preview, and matching parallel responses; no throughput, hosted-deployment, or standards-conformance claim.",
    }
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
