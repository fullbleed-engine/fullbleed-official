#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Execute the first-invoice notebook and inspect its actual output artifacts."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
import tempfile

import nbformat
from nbclient import NotebookClient
from jupyter_client import AsyncKernelManager
from jupyter_client.kernelspec import KernelSpecManager


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "examples" / "notebooks" / "first_invoice.ipynb"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "notebook-check")
    parser.add_argument("--skip-install", action="store_true", help="Use the already installed wheel")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    notebook = nbformat.read(SOURCE, as_version=4)
    nbformat.validate(notebook)
    for cell in notebook.cells:
        if cell.cell_type == "code" and (cell.outputs or cell.execution_count is not None):
            raise ValueError("Commit the notebook with empty outputs and execution counts")
        if args.skip_install and "install-fullbleed" in cell.metadata.get("tags", []):
            cell.metadata.tags.append("skip-execution")

    # Bind the kernel to this interpreter without touching the user's Jupyter config.
    with tempfile.TemporaryDirectory(prefix="fullbleed-notebook-kernel-") as kernel_dir:
        spec = Path(kernel_dir) / "fullbleed-check"
        spec.mkdir()
        (spec / "kernel.json").write_text(
            json.dumps({
                "argv": [sys.executable, "-m", "ipykernel_launcher", "-f", "{connection_file}"],
                "display_name": "Fullbleed notebook check",
                "language": "python",
            }),
            encoding="utf-8",
        )
        client = NotebookClient(
            notebook,
            km=AsyncKernelManager(
                kernel_name="fullbleed-check",
                kernel_spec_manager=KernelSpecManager(kernel_dirs=[kernel_dir]),
            ),
            timeout=180,
            kernel_name="fullbleed-check",
            resources={"metadata": {"path": str(out)}},
        )
        try:
            client.execute(cleanup_kc=True)
        finally:
            nbformat.write(notebook, out / "executed.ipynb")

    import fullbleed

    pdf = out / "output" / "invoice.pdf"
    inspection = dict(fullbleed.inspect_pdf(str(pdf)))
    extracted = fullbleed.extract_pdf_page_texts(str(pdf))
    text = "\n".join(page.get("text", "") for page in extracted["pages"])
    for marker in ["INV-1042", "Maple & Finch", "Design workshop", "USD 1,870.00"]:
        if marker not in text:
            raise AssertionError(f"Missing default invoice text: {marker}")
    if inspection["page_count"] != 1:
        raise AssertionError(f"Default invoice should fit one page: {inspection['page_count']}")
    if inspection["profile"].get("embedded_font_count", 0) < 1:
        raise AssertionError("The invoice must embed its bundled font")
    if inspection.get("warnings") or inspection.get("composition", {}).get("issues"):
        raise AssertionError("PDF inspection reported warnings or composition issues")
    previews = list((out / "output" / "preview").glob("*.png"))
    if len(previews) != 1 or previews[0].read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError("Expected one PNG preview of the default invoice")
    report = json.loads((out / "output" / "verification.json").read_text(encoding="utf-8"))
    if report["sha256"] != hashlib.sha256(pdf.read_bytes()).hexdigest():
        raise AssertionError("Verification digest does not match the delivered PDF")
    if report["text_checks"] != "passed" or report["repeat_render_identical"] is not True:
        raise AssertionError("Notebook's text and repeat-render checks did not pass")
    # Keep the full inspector result so font embedding and profile scope are reviewable.
    (out / "inspection.json").write_text(json.dumps(inspection, indent=2) + "\n", encoding="utf-8")
    result = {
        "ok": True,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "notebook": str(SOURCE.relative_to(ROOT)),
        "notebook_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "install_cell": "skipped; using installed wheel" if args.skip_install else "executed",
        "executed_code_cells": sum(cell.cell_type == "code" and cell.execution_count is not None for cell in notebook.cells),
        "pdf": str(pdf),
        "preview": str(previews[0]),
        "embedded_font_count": inspection["profile"]["embedded_font_count"],
        **report,
        "scope": "Jupyter execution and artifact checks; hosted Colab UI and standards conformance not evaluated.",
    }
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
