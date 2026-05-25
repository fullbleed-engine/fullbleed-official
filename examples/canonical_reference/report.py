from __future__ import annotations

import json
import os
import re
import tempfile
from importlib import resources
from pathlib import Path
from typing import Any

import fullbleed

from components import ArchitecturePage, AppendixPages, CoverPage, DataPages, PaginationPage, TimelinePage, VisualsPage
from components.fb_ui import Document, compile_document, validate_component_mount


ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parents[1]
DATA_PATH = ROOT / "data" / "reference.json"
OUTPUT_DIR = Path(os.getenv("FULLBLEED_OUTPUT_DIR", "") or (ROOT / "output")).resolve()

HTML_PATH = OUTPUT_DIR / "canonical_reference.html"
STANDALONE_HTML_PATH = OUTPUT_DIR / "canonical_reference_standalone.html"
CSS_ARTIFACT_PATH = OUTPUT_DIR / "canonical_reference.css"
PDF_PATH = OUTPUT_DIR / "canonical_reference.pdf"
PAGE_DATA_PATH = OUTPUT_DIR / "canonical_reference_page_data.json"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "canonical_reference_component_mount_validation.json"
CSS_LAYER_REPORT_PATH = OUTPUT_DIR / "canonical_reference_css_layers.json"
RUN_REPORT_PATH = OUTPUT_DIR / "canonical_reference_run_report.json"
JIT_PATH = OUTPUT_DIR / "canonical_reference.jit.jsonl"
PERF_PATH = OUTPUT_DIR / "canonical_reference.perf.jsonl"
PNG_STEM = "canonical_reference"

TITLE = "Fullbleed Canonical Reference"
RASTER_DATA_URI = (
    "data:image/png;base64,"
    "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAMUlEQVR4nGN4ZBsERJ92eAOR/"
    "PwuIFJP3QBEDDglMIUgSnFLYApBlOKWwBSCKMUpAQD9lF6hcMN4GwAAAABJRU5ErkJggg=="
)

CSS_LAYER_ORDER = [
    "styles/tokens.css",
    "components/styles/primitives.css",
    "components/styles/cover.css",
    "components/styles/architecture.css",
    "components/styles/data.css",
    "components/styles/visuals.css",
    "components/styles/appendix.css",
    "styles/report.css",
]

# Mirrors parser signals in src/style.rs where these declarations are parsed
# but currently have no static PDF effect.
NO_EFFECT_PROPERTIES = {
    "align-content",
    "align-self",
    "justify-items",
    "justify-self",
    "place-content",
    "place-items",
    "place-self",
    "row-gap",
    "column-gap",
    "flex-flow",
    "grid-template-rows",
    "grid-auto-columns",
    "grid-auto-rows",
    "grid-auto-flow",
    "grid-template-areas",
    "grid-template",
    "grid",
    "grid-row-start",
    "grid-row-end",
    "grid-column-start",
    "grid-column-end",
    "grid-row",
    "grid-column",
    "grid-area",
}

NORMALIZED_DISPLAY_VALUES = {
    "ruby",
    "ruby-base",
    "ruby-text",
    "ruby-base-container",
    "ruby-text-container",
    "table-column",
    "table-column-group",
}


def _env_truthy(name: str) -> bool:
    value = os.getenv(name, "").strip().lower()
    return value in {"1", "true", "yes", "on"}


def _env_int(name: str, default: int) -> int:
    raw = os.getenv(name, "").strip()
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        return default


def _shorten(text: Any, limit: int = 148) -> str:
    cleaned = re.sub(r"\s+", " ", str(text or "")).strip()
    if len(cleaned) <= limit:
        return cleaned
    return cleaned[: limit - 1].rstrip() + "..."


def load_reference_data(path: Path = DATA_PATH) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    required = ["document", "metrics", "pipeline", "scaffold", "css_layers", "page_plan", "feature_matrix"]
    missing = [key for key in required if key not in data]
    if missing:
        raise ValueError(f"reference data missing required keys: {', '.join(missing)}")
    return data


def load_coverage_snapshot() -> dict[str, Any]:
    status_path = REPO_ROOT / "_css_working" / "css_parity_status.json"
    fixture_path = REPO_ROOT / "_css_working" / "css_fixture_report.json"
    modules: list[dict[str, Any]] = []
    source = "fallback"

    if status_path.exists():
        source = "local report"
        status = json.loads(status_path.read_text(encoding="utf-8"))
        for module in status.get("modules", []):
            modules.append(
                {
                    "label": module.get("label", module.get("id", "")),
                    "status": module.get("status", "unknown"),
                    "priority": module.get("priority", "n/a"),
                    "notes": _shorten(module.get("notes", ""), 96),
                }
            )

    fixtures_label = "not run"
    fixtures_detail = "Run tools/run_css_fixture_suite.py to populate the local fixture report."
    fixtures_failed: int | str = "n/a"
    if fixture_path.exists():
        fixture_report = json.loads(fixture_path.read_text(encoding="utf-8"))
        summary = fixture_report.get("summary", {})
        total = int(summary.get("fixtures_total") or 0)
        passed = int(summary.get("fixtures_passed") or 0)
        fixtures_failed = int(summary.get("fixtures_failed") or 0)
        fixtures_label = f"{passed}/{total}"
        fixtures_detail = f"{passed} passing, {fixtures_failed} failing in the local fixture report."

    return {
        "summary": {
            "source": source,
            "modules_total": len(modules) if modules else "n/a",
            "fixtures_label": fixtures_label,
            "fixtures_detail": fixtures_detail,
            "fixtures_failed": fixtures_failed,
        },
        "modules": modules,
    }


def _selector_scope_ok(selector: str) -> bool:
    cleaned = selector.strip()
    if not cleaned:
        return True
    if "," in cleaned:
        return all(_selector_scope_ok(part) for part in cleaned.split(","))
    if cleaned.startswith("@"):
        return True
    if cleaned in {":root", "html", "body", "*"}:
        return True
    if cleaned.startswith("html ") or cleaned.startswith("body "):
        return True
    if '[data-fb-role="document-root"]' in cleaned:
        return True
    return False


def _scan_css_layer(layer: str, css: str) -> tuple[list[dict[str, str]], list[dict[str, str]]]:
    unscoped: list[dict[str, str]] = []
    no_effect: list[dict[str, str]] = []
    for block in css.split("}"):
        if "{" not in block:
            continue
        selector, body = block.split("{", 1)
        selector = selector.strip()
        if selector and not _selector_scope_ok(selector):
            unscoped.append({"layer": layer, "selector": selector})
        for prop, value in re.findall(r"([A-Za-z-]+)\s*:\s*([^;{}]+)", body):
            prop_l = prop.strip().lower()
            value_l = value.strip().lower()
            if prop_l in NO_EFFECT_PROPERTIES:
                no_effect.append({"layer": layer, "property": prop_l, "value": value.strip()})
            if prop_l == "display" and value_l in NORMALIZED_DISPLAY_VALUES:
                no_effect.append({"layer": layer, "property": prop_l, "value": value.strip()})
    return unscoped, no_effect


def load_css_layers() -> tuple[str, dict[str, Any]]:
    parts: list[str] = []
    layers: list[dict[str, Any]] = []
    unscoped_selectors: list[dict[str, str]] = []
    no_effect_declarations: list[dict[str, str]] = []

    for rel in CSS_LAYER_ORDER:
        path = ROOT / rel
        css = path.read_text(encoding="utf-8")
        parts.append(f"/* {rel} */\n{css}")
        unscoped, no_effect = _scan_css_layer(rel, css)
        unscoped_selectors.extend(unscoped)
        no_effect_declarations.extend(no_effect)
        layers.append(
            {
                "path": rel,
                "bytes": len(css.encode("utf-8")),
                "unscoped_selector_count": len(unscoped),
                "no_effect_declaration_count": len(no_effect),
            }
        )

    report = {
        "schema": "fullbleed.canonical_reference.css_layers.v1",
        "layer_count": len(layers),
        "layers": layers,
        "unscoped_selectors": unscoped_selectors,
        "no_effect_declarations": no_effect_declarations,
    }
    return "\n\n".join(parts), report


def _resolve_font_path() -> Path | None:
    candidates = [
        ROOT / "vendor" / "fonts" / "Inter-Variable.ttf",
        REPO_ROOT / "python" / "fullbleed_assets" / "fonts" / "Inter-Variable.ttf",
    ]
    for path in candidates:
        if path.exists():
            return path
    try:
        packaged = resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
        if packaged.is_file():
            return Path(str(packaged))
    except Exception:
        return None
    return None


def create_engine(*, debug: bool | None = None, debug_out: str | None = None, jit_mode: str | None = None):
    bundle = fullbleed.AssetBundle()
    font_path = _resolve_font_path()
    font_files: list[str] | None = None
    if font_path is not None:
        font_files = [str(font_path)]
        bundle.add_file(str(font_path), "font", name="Inter")

    bundle.add_file(str(ROOT / "assets" / "brand-mark.svg"), "svg", name="brand-mark.svg")
    bundle.add_file(str(ROOT / "assets" / "reference-pattern.svg"), "svg", name="reference-pattern.svg")

    debug_enabled = _env_truthy("FULLBLEED_DEBUG") if debug is None else bool(debug)
    debug_target = debug_out if debug_out is not None else (str(JIT_PATH) if debug_enabled else None)
    effective_jit_mode = jit_mode if jit_mode is not None else (os.getenv("FULLBLEED_JIT_MODE") or None)

    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        font_files=font_files,
        reuse_xobjects=True,
        svg_form_xobjects=True,
        svg_raster_fallback=True,
        unicode_support=True,
        shape_text=True,
        unicode_metrics=True,
        document_lang="en-US",
        document_title=TITLE,
        footer_first=(
            "Fullbleed canonical reference | manual page {page} of {pages} | "
            "cover page | page weight {sum:reference.weight}"
        ),
        footer_each=(
            "Fullbleed canonical reference | manual page {page} of {pages} | "
            "events {count:reference.event} | page weight {sum:reference.weight}"
        ),
        footer_last=(
            "Fullbleed canonical reference | final page {page} of {pages} | "
            "total events {total_count:reference.event} | total weight {total:reference.weight}"
        ),
        footer_x="0.48in",
        footer_y_from_bottom="0.2in",
        footer_font_name="Inter",
        footer_font_size=7.2,
        footer_color="#59677a",
        paginated_context={
            "reference.weight": "sum:2",
            "reference.event": "count",
            "reference.category": "every",
        },
        layout_strategy=os.getenv("FULLBLEED_LAYOUT_STRATEGY") or None,
        jit_mode=effective_jit_mode,
        debug=debug_enabled,
        debug_out=debug_target,
        perf=_env_truthy("FULLBLEED_PERF"),
        perf_out=str(PERF_PATH) if _env_truthy("FULLBLEED_PERF") else None,
    )
    if hasattr(engine, "document_css_href"):
        engine.document_css_href = CSS_ARTIFACT_PATH.name
    if hasattr(engine, "document_css_source_path"):
        engine.document_css_source_path = str(CSS_ARTIFACT_PATH)
    if hasattr(engine, "document_css_media"):
        engine.document_css_media = "all"
    if hasattr(engine, "document_css_required"):
        engine.document_css_required = True
    engine.register_bundle(bundle)
    return engine


def _build_pages() -> list[object]:
    data = load_reference_data()
    coverage = load_coverage_snapshot()
    return [
        CoverPage(data, coverage),
        ArchitecturePage(data),
        PaginationPage(data),
        *DataPages(data),
        VisualsPage(data, RASTER_DATA_URI),
        TimelinePage(data),
        *AppendixPages(data, coverage),
    ]


@Document(
    page="LETTER",
    margin="0in",
    title=TITLE,
    bootstrap=False,
    lang="en-US",
    css_href=CSS_ARTIFACT_PATH.name,
    css_source_path=str(CSS_ARTIFACT_PATH),
    css_media="all",
    css_required=True,
)
def App(_props=None):
    return _build_pages()


@Document(
    page="LETTER",
    margin="0in",
    title=TITLE,
    bootstrap=False,
    lang="en-US",
)
def RenderApp(_props=None):
    return _build_pages()


def build_html() -> str:
    return compile_document(RenderApp())


def _emit_standalone_html(html: str, css: str) -> str:
    standalone = re.sub(
        r'<link rel="stylesheet" href="[^"]+"(?: media="[^"]+")? />',
        "",
        html,
        count=1,
    )
    standalone = standalone.replace("</head>", f"<style>\n{css}\n</style></head>", 1)
    STANDALONE_HTML_PATH.write_text(standalone, encoding="utf-8")
    return standalone


def _strip_stylesheet_links(html: str) -> str:
    return re.sub(
        r'<link rel="stylesheet" href="[^"]+"(?: media="[^"]+")? />',
        "",
        html,
    )


def _emit_preview_png(engine: Any, html: str, css: str, *, dpi: int) -> list[str]:
    if hasattr(engine, "render_finalized_pdf_image_pages_to_dir") and PDF_PATH.exists():
        return list(
            engine.render_finalized_pdf_image_pages_to_dir(
                str(PDF_PATH),
                str(OUTPUT_DIR),
                dpi,
                PNG_STEM,
            )
            or []
        )
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(OUTPUT_DIR), dpi, PNG_STEM) or [])
    if hasattr(engine, "render_image_pages"):
        out: list[str] = []
        for index, image_bytes in enumerate(engine.render_image_pages(html, css, dpi) or [], start=1):
            path = OUTPUT_DIR / f"{PNG_STEM}_page{index}.png"
            path.write_bytes(image_bytes)
            out.append(str(path))
        return out
    return []


def _page_count(page_data: Any) -> int | None:
    if isinstance(page_data, dict):
        pages = page_data.get("pages")
        if isinstance(pages, list):
            return len(pages)
        if isinstance(page_data.get("page_count"), int):
            return int(page_data["page_count"])
    if isinstance(page_data, list):
        return len(page_data)
    return None


def _write_json(path: Path, payload: Any) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    strict_validate = _env_truthy("FULLBLEED_VALIDATE_STRICT")
    image_dpi = _env_int("FULLBLEED_IMAGE_DPI", 144)

    css, css_report = load_css_layers()
    _write_json(CSS_LAYER_REPORT_PATH, css_report)

    artifact = App()
    artifact_report = artifact.emit_artifacts(
        css=css,
        html_path=HTML_PATH,
        css_path=CSS_ARTIFACT_PATH,
        css_href=CSS_ARTIFACT_PATH.name,
    )
    linked_html = str(artifact_report["html"])
    html = compile_document(RenderApp())
    _emit_standalone_html(linked_html, css)

    if css_report["unscoped_selectors"]:
        print(f"[warn] Unscoped CSS selectors: {len(css_report['unscoped_selectors'])}")
        if strict_validate:
            raise SystemExit(2)
    if css_report["no_effect_declarations"]:
        print(f"[warn] Engine no-effect CSS declarations: {len(css_report['no_effect_declarations'])}")
        if strict_validate:
            raise SystemExit(2)

    with tempfile.NamedTemporaryFile(prefix="canonical_reference_mount_", suffix=".jit.jsonl", delete=False) as tmp:
        mount_debug_path = Path(tmp.name)
    try:
        validation_engine = create_engine(debug=True, debug_out=str(mount_debug_path), jit_mode="plan")
        mount_validation = validate_component_mount(
            engine=validation_engine,
            node_or_component=RenderApp,
            css=css,
            debug_log=str(mount_debug_path),
            title="canonical reference component mount",
            fail_on_overflow=False,
            fail_on_css_warnings=strict_validate,
            fail_on_known_loss=strict_validate,
            fail_on_html_asset_warning=True,
            fail_on_asset_resolution=True,
            fail_on_text_overlap=True,
            fail_on_flowable_overlap=True,
        )
    finally:
        mount_debug_path.unlink(missing_ok=True)

    _write_json(COMPONENT_VALIDATION_PATH, mount_validation)
    if not mount_validation.get("ok", False):
        print(f"[error] Component mount validation failed: {COMPONENT_VALIDATION_PATH}")
        raise SystemExit(2)

    engine = create_engine()
    page_data = None
    if hasattr(engine, "render_pdf_with_page_data"):
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        PDF_PATH.write_bytes(pdf_bytes)
        bytes_written = len(pdf_bytes)
        if page_data is not None:
            _write_json(PAGE_DATA_PATH, page_data)
    else:
        bytes_written = engine.render_pdf_to_file(html, css, str(PDF_PATH))

    png_paths = _emit_preview_png(engine, html, css, dpi=image_dpi)
    png_files = [Path(path) for path in png_paths]
    missing_pngs = [str(path) for path in png_files if not path.exists() or path.stat().st_size <= 0]

    page_count = _page_count(page_data)
    run_report = {
        "schema": "fullbleed.canonical_reference.run.v1",
        "ok": bool(PDF_PATH.exists() and bytes_written > 0 and png_paths and not missing_pngs),
        "artifacts": {
            "html": str(HTML_PATH),
            "standalone_html": str(STANDALONE_HTML_PATH),
            "css": str(CSS_ARTIFACT_PATH),
            "pdf": str(PDF_PATH),
            "page_data": str(PAGE_DATA_PATH) if PAGE_DATA_PATH.exists() else None,
            "png_pages": png_paths,
            "css_layers": str(CSS_LAYER_REPORT_PATH),
            "component_validation": str(COMPONENT_VALIDATION_PATH),
        },
        "pdf_bytes": bytes_written,
        "png_count": len(png_paths),
        "missing_pngs": missing_pngs,
        "page_count_from_data": page_count,
        "component_validation_ok": bool(mount_validation.get("ok", False)),
        "component_validation_warning_count": len(mount_validation.get("warnings") or []),
        "css_layer_warning_count": len(css_report["unscoped_selectors"]) + len(css_report["no_effect_declarations"]),
    }
    _write_json(RUN_REPORT_PATH, run_report)

    if not run_report["ok"]:
        print(f"[error] Render artifacts incomplete: {RUN_REPORT_PATH}")
        raise SystemExit(2)

    print(f"[ok] HTML: {HTML_PATH}")
    print(f"[ok] Standalone HTML: {STANDALONE_HTML_PATH}")
    print(f"[ok] CSS: {CSS_ARTIFACT_PATH}")
    print(f"[ok] PDF: {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] PNG pages: {len(png_paths)}")
    print(f"[ok] Page data: {PAGE_DATA_PATH if PAGE_DATA_PATH.exists() else 'not emitted'}")
    print(f"[ok] CSS layer report: {CSS_LAYER_REPORT_PATH}")
    print(f"[ok] Component validation: {COMPONENT_VALIDATION_PATH}")
    print(f"[ok] Run report: {RUN_REPORT_PATH}")


if __name__ == "__main__":
    main()
