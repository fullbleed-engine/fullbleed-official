from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

DEFAULT_INCLUDE_PATHS = [
    ".gitignore",
    "README.md",
    "SCAFFOLDING.md",
    "COMPLIANCE.md",
    "assets.lock.json",
    "report.py",
    "run_vdp_permutation_job.py",
    "i-9.pdf",
    "data",
    "styles",
    "components",
    "vendor",
    "tools/build_i9_fields.py",
]

EXCLUDED_DIR_NAMES = {
    "__pycache__",
    "output",
    "dist",
}

EXCLUDED_FILE_SUFFIXES = {".pyc", ".pyo"}


def _copy_tree(src: Path, dst: Path) -> list[str]:
    written: list[str] = []
    if src.is_file():
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
        written.append(str(dst))
        return written

    for path in sorted(src.rglob("*")):
        rel = path.relative_to(src)
        if any(part in EXCLUDED_DIR_NAMES for part in rel.parts):
            continue
        if path.is_dir():
            continue
        if path.suffix.lower() in EXCLUDED_FILE_SUFFIXES:
            continue
        out = dst / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, out)
        written.append(str(out))
    return written


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def _zip_dir(source_dir: Path, zip_path: Path) -> None:
    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for file_path in sorted(source_dir.rglob("*")):
            if file_path.is_dir():
                continue
            arcname = file_path.relative_to(source_dir).as_posix()
            zf.write(file_path, arcname=arcname)


def build_release(
    *,
    template_id: str,
    version: str,
    out_dir: Path,
    root_dir: str,
    homepage: str,
    smoke_command: str,
    expected_output: str,
) -> dict:
    timestamp = datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    out_dir.mkdir(parents=True, exist_ok=True)

    zip_name = f"{template_id}-v{version}.zip"
    zip_path = out_dir / zip_name

    with tempfile.TemporaryDirectory(prefix=f"fullbleed_{template_id}_") as temp_dir:
        stage_root = Path(temp_dir) / root_dir
        stage_root.mkdir(parents=True, exist_ok=True)

        files_written = 0
        for rel in DEFAULT_INCLUDE_PATHS:
            src = ROOT / rel
            if not src.exists():
                raise FileNotFoundError(f"required path missing: {src}")
            dst = stage_root / rel
            files_written += len(_copy_tree(src, dst))

        _zip_dir(Path(temp_dir), zip_path)

    sha = _sha256(zip_path)
    size_bytes = zip_path.stat().st_size

    release_block = {
        "version": version,
        "published_at": timestamp,
        "fullbleed_version_range": ">=0.2.6",
        "python_version_range": ">=3.10",
        "archive": {
            "url": f"https://github.com/fullbleed-engine/fullbleed-manifest/releases/download/{template_id}-v{version}/{zip_name}",
            "format": "zip",
            "sha256": sha,
            "size_bytes": size_bytes,
            "root_dir": root_dir,
        },
        "entrypoints": {
            "readme": "README.md",
            "run": "report.py",
        },
        "assets": {
            "lock_file": "assets.lock.json",
            "vendor_included": True,
        },
        "checks": {
            "smoke_command": smoke_command,
            "expected_outputs": [expected_output],
        },
        "deprecated": False,
    }

    template_block = {
        "id": template_id,
        "name": "I-9 Stamped VDP",
        "summary": "Per-record overlay + PDF template composition with conditional back pages.",
        "description": "Production-ready Fullbleed project template for high-volume VDP jobs.",
        "tags": ["vdp", "transactional", "templating", "duplex", "pdf"],
        "license": "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial",
        "maintainer": "fullbleed-engine",
        "homepage": homepage,
        "latest": version,
        "releases": [release_block],
    }

    metadata = {
        "schema": "fullbleed.template_release_package.v1",
        "ok": True,
        "template_id": template_id,
        "version": version,
        "root_dir": root_dir,
        "zip_path": str(zip_path.resolve()),
        "zip_name": zip_name,
        "sha256": sha,
        "size_bytes": size_bytes,
        "staged_files": files_written,
        "release": release_block,
        "template": template_block,
    }

    (out_dir / f"{template_id}-v{version}.release.json").write_text(
        json.dumps(metadata, indent=2),
        encoding="utf-8",
    )
    (out_dir / f"{template_id}-v{version}.manifest-entry.json").write_text(
        json.dumps(template_block, indent=2),
        encoding="utf-8",
    )
    return metadata


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Package form-i9 as a manifest-ready remote template release.")
    parser.add_argument("--template-id", default="i9-stamped-vdp")
    parser.add_argument("--version", default="1.0.0")
    parser.add_argument("--root-dir", default=None)
    parser.add_argument(
        "--out-dir",
        default=str(ROOT / "dist" / "releases"),
    )
    parser.add_argument(
        "--homepage",
        default="https://github.com/fullbleed-engine/fullbleed-manifest/tree/main/templates/i9-stamped-vdp",
    )
    parser.add_argument("--smoke-command", default="python report.py")
    parser.add_argument("--expected-output", default="output/report.pdf")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root_dir = args.root_dir or args.template_id
    metadata = build_release(
        template_id=str(args.template_id).strip(),
        version=str(args.version).strip(),
        out_dir=Path(args.out_dir).resolve(),
        root_dir=str(root_dir).strip(),
        homepage=str(args.homepage).strip(),
        smoke_command=str(args.smoke_command).strip(),
        expected_output=str(args.expected_output).strip(),
    )
    print(json.dumps(metadata, ensure_ascii=True))


if __name__ == "__main__":
    main()
