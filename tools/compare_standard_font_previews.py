#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare retained portability evidence from Windows and Linux CI jobs."""
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    reports = [(path, json.loads(path.read_text(encoding="utf-8")))
               for path in sorted(args.root.rglob("verification.json"))]
    assert reports, "No retained preview reports"
    assert {report["platform"] for _, report in reports} >= {"win32", "linux"}, "Windows and Linux are both required"
    assert any(report["masked_font_directories"] for _, report in reports), "Missing isolated Linux no-font run"
    expected = None
    versions = set()
    for path, report in reports:
        assert report["schema"] == "fullbleed.standard_font_portability.v1" and report["ok"], str(path)
        assert report["pdf_sources_unchanged"], str(path)
        actual = {row["name"]: [row["pixels_sha256"], row["pdf_sha256"], row["size"]] for row in report["cases"]}
        assert len(actual) == len(report["cases"]) == 68, str(path)
        versions.add(report["version"])
        if expected is None:
            expected = actual
        assert actual == expected, f"Pixel/PDF mismatch in {path}"
    assert len(versions) == 1, "Mixed engine versions"
    result = dict(ok=True, version=versions.pop(), cases=len(expected), reports=[str(path) for path, _ in reports])
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
