#!/usr/bin/env python
"""Smoke-test an installed Fullbleed wheel without importing test helpers."""
from __future__ import annotations

import argparse
import importlib.metadata
import importlib.resources
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


def run(expected_version: str) -> dict[str, Any]:
    import fullbleed

    installed_version = importlib.metadata.version("fullbleed")
    if installed_version != expected_version:
        raise AssertionError(
            f"expected fullbleed {expected_version}, found {installed_version}"
        )
    if fullbleed.SPDX_LICENSE_EXPRESSION != "MIT":
        raise AssertionError(
            f"expected MIT, found {fullbleed.SPDX_LICENSE_EXPRESSION!r}"
        )

    contract_resource = importlib.resources.files("fullbleed").joinpath(
        "agent_contract.json"
    )
    if not contract_resource.is_file():
        raise AssertionError("installed wheel is missing fullbleed/agent_contract.json")
    packaged_contract = json.loads(contract_resource.read_text(encoding="utf-8"))
    if packaged_contract.get("schema") != "fullbleed.agent_contract.v1":
        raise AssertionError("packaged agent contract has the wrong schema")
    if packaged_contract.get("product", {}).get("version") != expected_version:
        raise AssertionError("packaged agent contract version disagrees with the wheel")
    required_engine_fields = {
        "compiled_document",
        "compiled_reflow_bindings",
        "compiled_flow_compression_modes",
    }
    if not required_engine_fields.issubset(
        packaged_contract.get("capabilities", {}).get("engine", {})
    ):
        raise AssertionError("packaged agent contract omits compiled engine capabilities")
    llms_resource = importlib.resources.files("fullbleed").joinpath("llms.txt")
    if not llms_resource.is_file():
        raise AssertionError("installed wheel is missing fullbleed/llms.txt")
    packaged_llms = llms_resource.read_text(encoding="utf-8")
    if f"Fullbleed {expected_version}" not in packaged_llms:
        raise AssertionError("packaged llms.txt version disagrees with the wheel")

    engine = fullbleed.PdfEngine(svg_raster_fallback=True)
    html = (
        "<svg width='40' height='20' viewBox='0 0 40 20'>"
        "<rect width='40' height='20' fill='#d3212d'/>"
        "</svg>"
    )
    css = "@page { size: 40pt 20pt; margin: 0; } body { margin: 0; }"
    pdf = engine.render_pdf(html, css)
    if not pdf.startswith(b"%PDF"):
        raise AssertionError("render_pdf did not return a PDF")

    pages = list(engine.render_image_pages(html, css, 72))
    if len(pages) != 1 or not pages[0].startswith(b"\x89PNG\r\n\x1a\n"):
        raise AssertionError("render_image_pages did not return one PNG page")

    with tempfile.TemporaryDirectory(prefix="fullbleed-wheel-smoke-") as temp_dir:
        compliance = subprocess.run(
            [
                sys.executable,
                "-m",
                "fullbleed",
                "compliance",
                "--strict",
                "--json",
            ],
            cwd=temp_dir,
            check=False,
            capture_output=True,
            text=True,
        )
        runtime_contract_process = subprocess.run(
            [
                sys.executable,
                "-m",
                "fullbleed",
                "agent-contract",
                "--format",
                "json",
            ],
            cwd=temp_dir,
            check=False,
            capture_output=True,
            text=True,
        )
        runtime_llms_process = subprocess.run(
            [
                sys.executable,
                "-m",
                "fullbleed",
                "agent-contract",
                "--format",
                "llms",
            ],
            cwd=temp_dir,
            check=False,
            capture_output=True,
            text=True,
        )
    if compliance.returncode != 0:
        raise AssertionError(
            "installed-package compliance failed outside the source tree:\n"
            + compliance.stdout
            + compliance.stderr
        )
    compliance_report = json.loads(compliance.stdout)
    if compliance_report.get("license", {}).get("spdx_expression") != "MIT":
        raise AssertionError("installed-package compliance did not report MIT")
    if runtime_contract_process.returncode != 0:
        raise AssertionError(
            "installed runtime agent contract failed outside the source tree:\n"
            + runtime_contract_process.stdout
            + runtime_contract_process.stderr
        )
    runtime_contract = json.loads(runtime_contract_process.stdout)
    if runtime_contract != packaged_contract:
        raise AssertionError(
            "packaged agent contract disagrees with the installed runtime"
        )
    if runtime_llms_process.returncode != 0:
        raise AssertionError(
            "installed runtime llms.txt failed outside the source tree:\n"
            + runtime_llms_process.stdout
            + runtime_llms_process.stderr
        )
    if runtime_llms_process.stdout.replace("\r\n", "\n") != packaged_llms.replace(
        "\r\n", "\n"
    ):
        raise AssertionError("packaged llms.txt disagrees with the installed runtime")

    return {
        "ok": True,
        "version": installed_version,
        "license": fullbleed.SPDX_LICENSE_EXPRESSION,
        "module": str(Path(fullbleed.__file__).resolve()),
        "pdf_bytes": len(pdf),
        "png_bytes": len(pages[0]),
        "compliance_files": compliance_report["files"],
        "agent_contract_schema": packaged_contract["schema"],
        "agent_contract_tools": len(packaged_contract["tool_adapter"]["tools"]),
        "llms_txt": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Smoke-test an installed Fullbleed distribution"
    )
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    result = run(args.expected_version)
    if args.json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(
            "Fullbleed wheel smoke passed: "
            f"{result['version']} ({result['license']}), "
            f"{result['pdf_bytes']} PDF bytes, {result['png_bytes']} PNG bytes"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
