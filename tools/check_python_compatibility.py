#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check retained interpreter tests and compare their PDFs/previews with stable Python."""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import platform
import struct
import sys
import sysconfig
import xml.etree.ElementTree as ET
from pathlib import Path


RENDER_CHECKS = ("border-image-slices", "base14-preview", "gradient-preview")


def compare_renderings(reference: Path, candidate: Path, expected_version: str) -> dict:
    result = {}
    for name in RENDER_CHECKS:
        roots = (reference / name, candidate / name)
        for root in roots:
            report = json.loads((root / "verification.json").read_text(encoding="utf-8"))
            if report.get("ok") is not True:
                raise ValueError(f"Unsuccessful render check: {root}")
            if report.get("version", report.get("fullbleed_version")) != expected_version:
                raise ValueError(f"Engine version mismatch: {root}")
        inventories = [
            {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
             for path in sorted(root.rglob("*")) if path.suffix in {".pdf", ".png"}}
            for root in roots
        ]
        expected, actual = inventories
        if not expected or expected.keys() != actual.keys():
            raise ValueError(f"Missing or unexpected PDF/PNG files: {name}")
        mismatches = [name for name in expected if expected[name] != actual[name]]
        if mismatches:
            raise ValueError(f"PDF/PNG bytes differ in {name}: {mismatches}")
        result[name] = {"files": len(actual), "sha256": actual}
    return result


def check_test_results(path: Path, allowed_skips: set[str]) -> dict:
    root = ET.parse(path).getroot()
    suites = [root] if root.tag == "testsuite" else list(root.iter("testsuite"))
    cases = list(root.iter("testcase"))
    if not suites or not cases or sum(int(suite.get("tests", "0")) for suite in suites) != len(cases):
        raise ValueError("Missing or incomplete Python test results")
    if any(int(suite.get(field, "0")) for suite in suites for field in ("errors", "failures")):
        raise ValueError("Python tests contain failures or errors")
    if any(case.find(field) is not None for case in cases for field in ("failure", "error")):
        raise ValueError("Python tests contain failure or error records")
    skipped = []
    for case in cases:
        skip = case.find("skipped")
        if skip is not None:
            identifier = f"{case.get('classname')}.{case.get('name')}"
            if identifier not in allowed_skips:
                raise ValueError(f"Unexpected skipped test: {identifier}")
            skipped.append({"test": identifier, "reason": skip.get("message", "")})
    if sum(int(suite.get("skipped", "0")) for suite in suites) != len(skipped):
        raise ValueError("Skipped-test totals disagree with their records")
    return {"tests": len(cases), "passed": len(cases) - len(skipped), "skipped": skipped}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-python", required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True,
                        help="Existing render outputs and python-tests.xml; receives verification.json")
    parser.add_argument("--allow-skipped-test", action="append", default=[])
    args = parser.parse_args()
    result = {"schema": "fullbleed.python_compatibility.v1", "ok": False,
              "python": {"version": sys.version, "version_info": list(sys.version_info),
                         "implementation": platform.python_implementation(), "platform": sys.platform,
                         "architecture": platform.machine(), "pointer_bits": struct.calcsize("P") * 8,
                         "py_gil_disabled": sysconfig.get_config_var("Py_GIL_DISABLED") or 0},
              "expected_python": args.expected_python, "fullbleed": args.expected_version,
              "scope": "This exact standard-GIL interpreter and platform only. Repository tests use local Python wrappers and the installed native extension; isolated smoke checks exercise the installed wheel. No free-threaded or other-interpreter claim."}
    args.out.mkdir(parents=True, exist_ok=True)
    try:
        if platform.python_implementation() != "CPython" or platform.python_version() != args.expected_python:
            raise ValueError(f"Expected CPython {args.expected_python}, found {sys.version}")
        if result["python"]["py_gil_disabled"]:
            raise ValueError("This check covers standard-GIL CPython only")
        installed = importlib.metadata.distribution("fullbleed")
        if installed.version != args.expected_version or installed.requires:
            raise ValueError("Unexpected Fullbleed version or runtime dependencies")
        import fullbleed
        import fullbleed._fullbleed as native
        result["installed_package"] = {"python_module": fullbleed.__file__, "native_module": native.__file__,
                                       "requires_dist": installed.requires or []}
        result["python_tests"] = check_test_results(args.out / "python-tests.xml", set(args.allow_skipped_test))
        result["stable_python_render_comparison"] = compare_renderings(args.reference, args.out, args.expected_version)
        result["ok"] = True
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        (args.out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "python": args.expected_python, "fullbleed": args.expected_version,
                      "python_tests": result["python_tests"],
                      "matching_pdf_png_files": sum(item["files"] for item in result["stable_python_render_comparison"].values())}))


if __name__ == "__main__":
    main()
