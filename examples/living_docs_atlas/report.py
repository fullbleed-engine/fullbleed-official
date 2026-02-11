#!/usr/bin/env python
"""Engine-first renderer for the Living Docs Atlas example."""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import List

import fullbleed


PROJECT_ROOT = Path(__file__).resolve().parent
BUILD_DIR = PROJECT_ROOT / "build"
OUTPUT_DIR = PROJECT_ROOT / "output"
VENDOR_DIR = PROJECT_ROOT / "vendor"
VENDOR_CSS = VENDOR_DIR / "css"

SCRIPT_DIR = PROJECT_ROOT / "scripts"
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
from build_inputs import build_inputs  # pylint: disable=wrong-import-position


def create_engine() -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0.45in",
        reuse_xobjects=True,
        unicode_support=True,
        shape_text=True,
        unicode_metrics=True,
    )


def _resolve_bootstrap_path() -> Path:
    candidates = [
        VENDOR_CSS / "bootstrap.min.css",
        VENDOR_DIR / "bootstrap.min.css",
    ]
    for vendored in candidates:
        if vendored.exists():
            return vendored
    try:
        import fullbleed_assets  # pylint: disable=import-error

        return Path(fullbleed_assets.asset_path("bootstrap.min.css"))
    except Exception:
        raise FileNotFoundError(
            "bootstrap.min.css not found in vendor/css and no bundled fullbleed_assets fallback available."
        )


def _register_assets(bundle: fullbleed.AssetBundle, installed_font_files: List[str], bootstrap_path: Path) -> None:
    bundle.add_file(str(bootstrap_path), "css", name="bootstrap-5.0.0")
    for rel in installed_font_files:
        path = PROJECT_ROOT / rel
        if path.exists():
            bundle.add_file(str(path), "font", name=path.stem)


def main() -> int:
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    generated = build_inputs(font_url_prefix="vendor/fonts", write_files=True)
    summary = generated["summary"]
    html = generated["html"]
    css = generated["css"]

    bootstrap_path = _resolve_bootstrap_path()
    bootstrap_css = bootstrap_path.read_text(encoding="utf-8")

    bundle = fullbleed.AssetBundle()
    _register_assets(bundle, summary.get("installed_font_files", []), bootstrap_path)

    engine = create_engine()
    engine.register_bundle(bundle)

    out_pdf = OUTPUT_DIR / "living_docs_atlas.engine.pdf"
    css_merged = bootstrap_css + "\n\n" + css
    bytes_written = engine.render_pdf_to_file(html, css_merged, str(out_pdf))

    result = {
        "schema": "fullbleed.docs_engine_render.v1",
        "ok": True,
        "bytes_written": bytes_written,
        "output_pdf": str(out_pdf),
        "font_installed": summary.get("font_installed"),
        "font_missing": summary.get("font_missing"),
        "bootstrap_css": str(bootstrap_path),
    }
    (BUILD_DIR / "engine_render_summary.json").write_text(
        json.dumps(result, ensure_ascii=True, indent=2), encoding="utf-8"
    )
    print(json.dumps(result, ensure_ascii=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
