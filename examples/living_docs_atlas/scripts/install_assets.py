#!/usr/bin/env python
"""Install bootstrap + curated fonts into vendor/ for this docs project."""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path
from typing import Dict, List, Tuple


PROJECT_ROOT = Path(__file__).resolve().parents[1]
DATA_PATH = PROJECT_ROOT / "data" / "font_catalog.json"
VENDOR_DIR = PROJECT_ROOT / "vendor"
VENDOR_FONTS = VENDOR_DIR / "fonts"
VENDOR_CSS = VENDOR_DIR / "css"
BUILD_DIR = PROJECT_ROOT / "build"


def _resolve_fullbleed_cmd() -> List[str]:
    binary = shutil.which("fullbleed")
    if binary:
        return [binary]
    return [sys.executable, "-m", "fullbleed_cli.cli"]


def _load_catalog() -> List[Dict]:
    data = json.loads(DATA_PATH.read_text(encoding="utf-8"))
    fonts = data.get("fonts", [])
    if not isinstance(fonts, list):
        raise ValueError("font_catalog.json is missing 'fonts' list")
    return fonts


def _run_json(cmd: List[str], cwd: Path) -> Tuple[int, Dict, str]:
    proc = subprocess.run(cmd, cwd=str(cwd), capture_output=True, text=True, check=False)
    payload = {}
    stdout = (proc.stdout or "").strip()
    if stdout:
        for line in reversed(stdout.splitlines()):
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
                break
            except json.JSONDecodeError:
                continue
    return proc.returncode, payload, proc.stderr or ""


def _file_sha256(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def _safe_license_name(package: str) -> str:
    chars = []
    for ch in package.lower():
        if ch.isalnum() or ch in {"-", "_"}:
            chars.append(ch)
        else:
            chars.append("-")
    slug = "".join(chars).strip("-")
    return f"LICENSE.{slug}.txt"


def _download_binary(url: str, out_path: Path) -> int:
    with urllib.request.urlopen(url, timeout=120) as response:
        payload = response.read()
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_bytes(payload)
    return len(payload)


def _install_from_catalog(step: Dict, expected_path: Path) -> Tuple[bool, Dict | None, str]:
    font_url = str(step.get("font_url", "")).strip()
    if not font_url:
        return False, None, "no font_url in catalog metadata"

    package = str(step.get("package", "unknown"))
    version = str(step.get("version", "unknown"))
    license_name = step.get("license")
    license_url = str(step.get("license_url", "")).strip()

    try:
        size_bytes = _download_binary(font_url, expected_path)
    except Exception as exc:
        return False, None, f"catalog font download failed: {exc}"

    license_file: Path | None = None
    if license_url:
        try:
            license_file = expected_path.parent / _safe_license_name(package)
            _download_binary(license_url, license_file)
        except Exception as exc:
            return False, None, f"catalog license download failed: {exc}"

    payload = {
        "schema": "fullbleed.assets_install.v1",
        "ok": True,
        "name": package.lstrip("@"),
        "version": version,
        "installed_to": str(expected_path),
        "size_bytes": size_bytes,
        "sha256": _file_sha256(expected_path),
        "license": license_name,
        "license_file": str(license_file) if license_file else None,
        "source": "catalog-fallback",
    }
    return True, payload, ""


def _normalize_legacy_layout(package: str, expected_path: Path, payload: Dict) -> Dict:
    normalized = dict(payload)
    candidates = []
    installed_to = normalized.get("installed_to")
    if installed_to:
        candidates.append(Path(installed_to))
    candidates.append(VENDOR_DIR / expected_path.name)

    if not expected_path.exists():
        for candidate in candidates:
            if candidate.exists() and candidate.resolve() != expected_path.resolve():
                expected_path.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(candidate), str(expected_path))
                normalized["normalized_from"] = str(candidate)
                break

    if expected_path.exists():
        normalized["installed_to"] = str(expected_path)

    legacy_license = VENDOR_DIR / "LICENSE.txt"
    if legacy_license.exists() and normalized.get("license"):
        normalized_license = expected_path.parent / _safe_license_name(package)
        if not normalized_license.exists():
            normalized_license.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(legacy_license), str(normalized_license))
        normalized["license_file"] = str(normalized_license)

    return normalized


def main() -> int:
    parser = argparse.ArgumentParser(description="Install docs assets into vendor/")
    parser.add_argument("--reinstall", action="store_true", help="Reinstall assets even if files exist.")
    parser.add_argument("--skip-bootstrap", action="store_true", help="Skip @bootstrap install.")
    parser.add_argument("--dry-run", action="store_true", help="Print actions without executing.")
    parser.add_argument("--limit", type=int, default=0, help="Install only the first N planned packages (0 = all).")
    args = parser.parse_args()

    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    VENDOR_FONTS.mkdir(parents=True, exist_ok=True)
    VENDOR_CSS.mkdir(parents=True, exist_ok=True)

    base_cmd = _resolve_fullbleed_cmd()
    fonts = _load_catalog()

    install_plan: List[Dict] = []
    if not args.skip_bootstrap:
        install_plan.append(
            {
                "package": "@bootstrap",
                "expected_path": str((VENDOR_CSS / "bootstrap.min.css").resolve()),
                "kind": "css",
                "version": "5.0.0",
            }
        )
    for font in fonts:
        install_plan.append(
            {
                "package": font["package"],
                "expected_path": str((VENDOR_FONTS / font["filename"]).resolve()),
                "kind": "font",
                "version": font.get("version", "unknown"),
                "font_url": font.get("font_url"),
                "license_url": font.get("license_url"),
                "license": font.get("license"),
            }
        )
    if args.limit > 0:
        install_plan = install_plan[: args.limit]

    results = []
    failures = []
    installed = 0
    skipped = 0
    package_results: Dict[str, Dict] = {}

    for step in install_plan:
        expected_path = Path(step["expected_path"])
        package = step["package"]
        if expected_path.exists() and not args.reinstall:
            skipped += 1
            results.append({"package": package, "status": "skipped", "installed_to": str(expected_path)})
            package_results[package] = {"installed_to": str(expected_path)}
            continue

        cmd = base_cmd + [
            "assets",
            "install",
            package,
            "--vendor",
            str(VENDOR_DIR),
            "--json",
        ]

        if args.dry_run:
            results.append({"package": package, "status": "dry-run", "command": cmd})
            continue

        rc, payload, stderr = _run_json(cmd, cwd=VENDOR_DIR)
        if rc == 0 and payload.get("ok", True):
            normalized_payload = _normalize_legacy_layout(package, expected_path, payload)
            installed += 1
            results.append({"package": package, "status": "installed", "result": normalized_payload})
            package_results[package] = normalized_payload
        else:
            fallback_ok, fallback_payload, fallback_error = _install_from_catalog(step, expected_path)
            if fallback_ok and fallback_payload:
                installed += 1
                results.append(
                    {
                        "package": package,
                        "status": "installed",
                        "result": fallback_payload,
                        "note": "catalog fallback used after CLI install failure",
                    }
                )
                package_results[package] = fallback_payload
                continue
            failures.append(
                {
                    "package": package,
                    "returncode": rc,
                    "payload": payload,
                    "stderr": stderr,
                    "fallback_error": fallback_error,
                }
            )
            results.append({"package": package, "status": "failed"})

    lock_packages = []
    for step in install_plan:
        package = step["package"]
        expected_path = Path(step["expected_path"])
        if not expected_path.exists():
            continue
        package_result = package_results.get(package, {})
        name = package_result.get("name") or package.lstrip("@")
        version = package_result.get("version") or step.get("version")
        sha = package_result.get("sha256") or _file_sha256(expected_path)
        rel = expected_path.relative_to(PROJECT_ROOT).as_posix()
        lock_packages.append(
            {
                "name": name,
                "version": version,
                "kind": step.get("kind", "font"),
                "files": [{"path": rel, "sha256": sha}],
            }
        )
    lock_payload = {"schema": 1, "packages": lock_packages}
    (PROJECT_ROOT / "assets.lock.json").write_text(
        json.dumps(lock_payload, ensure_ascii=True, indent=2), encoding="utf-8"
    )

    summary = {
        "schema": "fullbleed.docs_assets_install_report.v1",
        "project": str(PROJECT_ROOT),
        "dry_run": args.dry_run,
        "planned": len(install_plan),
        "installed": installed,
        "skipped": skipped,
        "failed": len(failures),
        "results": results,
        "failures": failures,
        "lock_file": str(PROJECT_ROOT / "assets.lock.json"),
        "lock_packages": len(lock_packages),
    }

    report_path = BUILD_DIR / "assets_install_report.json"
    report_path.write_text(json.dumps(summary, ensure_ascii=True, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=True, indent=2))

    if failures and not args.dry_run:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
