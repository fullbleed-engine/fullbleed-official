#!/usr/bin/env python
"""Audit remote font licenses for fullbleed CLI.

Generates:
- FONT_LICENSE_AUDIT.json
- FONT_LICENSE_AUDIT.md

Exits with non-zero status when any row fails validation.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from pathlib import Path
from typing import Dict, List, Tuple


ALLOWED_LICENSES = {"OFL-1.1", "Apache-2.0", "UFL-1.0", "MIT"}
LICENSE_MARKERS = {
    "OFL-1.1": ("open font license", "sil open font license"),
    "Apache-2.0": ("apache license", "version 2.0"),
    "UFL-1.0": ("ubuntu font licence", "ubuntu font license"),
    "MIT": ("mit license",),
}


def _fetch_head_or_get(url: str) -> Tuple[bool, bytes]:
    """Check reachability and fetch a small payload."""
    try:
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=25):
            return True, b""
    except Exception:
        try:
            with urllib.request.urlopen(url, timeout=25) as resp:
                return True, resp.read(65536)
        except Exception:
            return False, b""


def _load_remote_assets(repo_root: Path) -> Dict[str, Dict]:
    sys.path.insert(0, str((repo_root / "fullbleed_cli" / "src").resolve()))
    from fullbleed_cli import assets  # pylint: disable=import-error

    return assets.REMOTE_ASSETS


def _validate_row(name: str, meta: Dict) -> Dict:
    license_name = meta.get("license")
    font_url = str(meta.get("url", ""))
    license_url = str(meta.get("license_url", ""))

    font_url_ok, _ = _fetch_head_or_get(font_url)
    license_url_ok, license_payload = _fetch_head_or_get(license_url)

    if license_url_ok and not license_payload:
        try:
            with urllib.request.urlopen(license_url, timeout=25) as resp:
                license_payload = resp.read(65536)
        except Exception:
            license_payload = b""

    allowed_license = license_name in ALLOWED_LICENSES
    markers = LICENSE_MARKERS.get(license_name, ())
    text = license_payload.decode("utf-8", errors="ignore").lower()
    license_text_ok = bool(markers) and any(marker in text for marker in markers)

    ok = allowed_license and font_url_ok and license_url_ok and license_text_ok
    return {
        "name": name,
        "kind": meta.get("kind"),
        "version": meta.get("version"),
        "license": license_name,
        "allowed_license": allowed_license,
        "font_url": font_url,
        "font_url_ok": font_url_ok,
        "license_url": license_url,
        "license_url_ok": license_url_ok,
        "license_text_ok": license_text_ok,
        "status": "PASS" if ok else "FAIL",
    }


def _write_reports(repo_root: Path, audit_date: str, rows: List[Dict], issues: List[Dict]) -> None:
    summary = {
        "audit_date": audit_date,
        "remote_assets_count": len(rows),
        "allowed_license_set": sorted(ALLOWED_LICENSES),
        "pass_count": sum(1 for row in rows if row["status"] == "PASS"),
        "fail_count": sum(1 for row in rows if row["status"] == "FAIL"),
    }
    payload = {"summary": summary, "rows": rows, "issues": issues}
    (repo_root / "FONT_LICENSE_AUDIT.json").write_text(
        json.dumps(payload, ensure_ascii=True, indent=2), encoding="utf-8"
    )

    lines = [
        "# Font License Audit",
        "",
        f"Audit date: {audit_date}",
        "",
        "Scope: `REMOTE_ASSETS` in `fullbleed_cli/src/fullbleed_cli/assets.py`.",
        "",
        "Method:",
        "- Checked each font URL is reachable.",
        "- Checked each license URL is reachable.",
        "- Checked license text contains expected marker for declared license.",
        "- Enforced allowlist for AGPL distribution compatibility review: `OFL-1.1`, `Apache-2.0`, `UFL-1.0`, `MIT`.",
        "",
        "Result:",
        f"- Total fonts: {summary['remote_assets_count']}",
        f"- Passed: {summary['pass_count']}",
        f"- Failed: {summary['fail_count']}",
        "",
        "| Font | Kind | Version | License | Font URL | License URL | Status |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        lines.append(
            f"| `{row['name']}` | `{row['kind']}` | `{row['version']}` | `{row['license']}` | "
            f"{row['font_url']} | {row['license_url']} | `{row['status']}` |"
        )
    lines.append("")
    if issues:
        lines.append("## Issues")
        for issue in issues:
            lines.append(f"- `{issue['name']}`: {json.dumps(issue, ensure_ascii=True)}")
    else:
        lines.append("No license or source integrity issues detected in this pass.")
    lines.append("")
    (repo_root / "FONT_LICENSE_AUDIT.md").write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Audit fullbleed remote font licenses")
    parser.add_argument("--date", required=True, help="Audit date (YYYY-MM-DD)")
    parser.add_argument("--repo-root", default=".", help="Repository root path")
    args = parser.parse_args()

    repo_root = Path(args.repo_root).resolve()
    remote_assets = _load_remote_assets(repo_root)

    rows: List[Dict] = []
    issues: List[Dict] = []
    for name, meta in sorted(remote_assets.items(), key=lambda item: item[0]):
        row = _validate_row(name, meta)
        rows.append(row)
        if row["status"] != "PASS":
            issues.append(
                {
                    "name": row["name"],
                    "license": row["license"],
                    "allowed_license": row["allowed_license"],
                    "font_url_ok": row["font_url_ok"],
                    "license_url_ok": row["license_url_ok"],
                    "license_text_ok": row["license_text_ok"],
                }
            )

    _write_reports(repo_root, args.date, rows, issues)
    summary = {
        "audit_date": args.date,
        "remote_assets_count": len(rows),
        "pass_count": sum(1 for row in rows if row["status"] == "PASS"),
        "fail_count": len(issues),
    }
    print(json.dumps(summary, ensure_ascii=True))
    return 1 if issues else 0


if __name__ == "__main__":
    raise SystemExit(main())
