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

import fullbleed

from invoice import load_invoice, render_invoice


def inspect(path: Path, markers: list[str]) -> dict:
    report = dict(fullbleed.inspect_pdf(str(path)))
    text = "\n".join(
        page.get("text", "") for page in fullbleed.extract_pdf_page_texts(str(path))["pages"]
    )
    for marker in markers:
        assert marker in text, (path.name, "missing PDF text", marker)
    assert report["page_count"] == 1, (path.name, "unexpected page count")
    assert report["profile"]["embedded_font_count"] >= 1, (path.name, "font not embedded")
    assert not report["warnings"], (path.name, report["warnings"])
    assert not report["composition"]["issues"], (path.name, report["composition"])
    return report


def check_client(name: str, client, out: Path) -> dict:
    response = client.get("/invoices/INV-1042.pdf")
    assert response.status_code == 200, (name, response.status_code)
    assert response.headers["Content-Type"] == "application/pdf", name
    assert response.headers["Content-Disposition"] == 'attachment; filename="invoice.pdf"', name
    assert response.headers["Cache-Control"] == "private, no-store", name
    content = response.data if name == "flask" else response.content
    pdf_path = out / f"{name}.pdf"
    pdf_path.write_bytes(content)
    report = inspect(pdf_path, ["INV-1042", "Maple & Finch", "Design workshop", "USD 1,870.00"])
    repeat = client.get("/invoices/INV-1042.pdf")
    assert content == (repeat.data if name == "flask" else repeat.content), name
    assert client.get("/invoices/DOES-NOT-EXIST.pdf").status_code == 404, name
    return {
        "framework": name,
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
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/web-framework-check"))
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    from fastapi.testclient import TestClient
    from fastapi_app import app as fastapi_app
    from flask_app import app as flask_app

    with TestClient(fastapi_app) as client:
        results = [check_client("fastapi", client, out)]
        schema = client.get("/openapi.json").json()
        assert "application/pdf" in schema["paths"]["/invoices/{invoice_id}.pdf"]["get"]["responses"]["200"]["content"]
    with flask_app.test_client() as client:
        results.append(check_client("flask", client, out))

    os.environ.setdefault("DJANGO_SETTINGS_MODULE", "django_app")
    import django
    from django.test import Client

    django.setup()
    results.append(check_client("django", Client(), out))
    assert len({result["sha256"] for result in results}) == 1, "Frameworks changed the PDF bytes"

    # Exercise literal markup-like customer text independently of the fixed HTTP sample.
    data = load_invoice("INV-1042")
    assert data is not None
    data["customer"] = "North <East> & Partners"
    escaped = out / "escaped-text.pdf"
    escaped.write_bytes(render_invoice(data))
    inspect(escaped, [data["customer"], "USD 1,870.00"])

    previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages_to_dir(
        str(out / "fastapi.pdf"), str(out / "preview"), 110, "invoice"
    )
    assert len(previews) == 1 and Path(previews[0]).read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    report = {
        "ok": True,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "versions": {name: metadata.version(name) for name in ["fullbleed", "fastapi", "Flask", "Django", "httpx2"]},
        "frameworks": results,
        "all_framework_pdf_bytes_identical": True,
        "literal_customer_text_check": "passed",
        "fastapi_openapi_pdf_type": "passed",
        "preview": str(previews[0]),
        "scope": "Framework test clients, PDF content, headers, font embedding, and repeat requests; no throughput or standards-conformance claim.",
    }
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
