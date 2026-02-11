#!/usr/bin/env python
"""Validate project licensing files for dual-license integrity.

This check is intentionally independent of package build/install steps so it can
be used as a fast CI gate.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Dict, List


SPDX_EXPRESSION = "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial"
LICENSE_HEADER = "GNU AFFERO GENERAL PUBLIC LICENSE"
LICENSE_FORBIDDEN = ("Apache License",)
COPYRIGHT_MARKERS = (
    "dual-licensed",
    "AGPL-3.0-only",
    "LicenseRef-Fullbleed-Commercial",
    "LICENSE",
    "LICENSING.md",
)
LICENSING_MARKERS = ("dual-licensed", SPDX_EXPRESSION)
MOJIBAKE_MARKERS = ("â€”", "â€™", "â€œ", "â€", "\ufffd")


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _fail(code: str, target: str, message: str) -> Dict[str, str]:
    return {"code": code, "target": target, "message": message}


def run(repo_root: Path) -> Dict:
    flags: List[Dict[str, str]] = []
    files = {
        "license": repo_root / "LICENSE",
        "copyright": repo_root / "COPYRIGHT",
        "licensing_guide": repo_root / "LICENSING.md",
        "third_party_notice": repo_root / "THIRD_PARTY_LICENSES.md",
    }

    for key, path in files.items():
        if not path.exists():
            flags.append(_fail("LIC_MISSING_NOTICE", str(path), f"Missing required file: {key}"))

    if files["license"].exists():
        text = _read(files["license"])
        if LICENSE_HEADER not in text:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["license"]),
                    f"LICENSE missing sentinel header: {LICENSE_HEADER}",
                )
            )
        bad = [m for m in LICENSE_FORBIDDEN if m.lower() in text.lower()]
        if bad:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["license"]),
                    "LICENSE contains disallowed marker(s): " + ", ".join(bad),
                )
            )

    if files["copyright"].exists():
        text = _read(files["copyright"])
        if SPDX_EXPRESSION not in text:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["copyright"]),
                    "COPYRIGHT missing SPDX expression: " + SPDX_EXPRESSION,
                )
            )
        lowered = text.lower()
        missing = [m for m in COPYRIGHT_MARKERS if m.lower() not in lowered]
        if missing:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["copyright"]),
                    "COPYRIGHT missing required marker(s): " + ", ".join(missing),
                )
            )

    if files["licensing_guide"].exists():
        text = _read(files["licensing_guide"])
        lowered = text.lower()
        missing = [m for m in LICENSING_MARKERS if m.lower() not in lowered]
        if missing:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["licensing_guide"]),
                    "LICENSING.md missing required marker(s): " + ", ".join(missing),
                )
            )
        mojibake = [m for m in MOJIBAKE_MARKERS if m in text]
        if mojibake:
            flags.append(
                _fail(
                    "LIC_POLICY_MISMATCH",
                    str(files["licensing_guide"]),
                    "LICENSING.md contains encoding artifacts: " + ", ".join(mojibake),
                )
            )

    return {
        "schema": "fullbleed.license_integrity.v1",
        "ok": len(flags) == 0,
        "spdx_expression": SPDX_EXPRESSION,
        "files": {k: str(v) for k, v in files.items()},
        "flags": flags,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Check dual-license file integrity.")
    parser.add_argument(
        "--repo-root",
        default=str(Path(__file__).resolve().parents[1]),
        help="Repository root path (defaults to parent of tools/).",
    )
    parser.add_argument("--json", action="store_true", help="Print JSON output.")
    args = parser.parse_args()

    report = run(Path(args.repo_root))
    if args.json:
        print(json.dumps(report, ensure_ascii=True))
    else:
        print(f"schema: {report['schema']}")
        print(f"ok: {report['ok']}")
        if report["flags"]:
            print("flags:")
            for flag in report["flags"]:
                print(f"  - {flag['code']}: {flag['message']}")
        else:
            print("flags: []")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

