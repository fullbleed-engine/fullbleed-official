from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any

import fitz
import fullbleed

from components.fb_ui import Document, compile_document, validate_component_mount
from components.i9_overlay import I9Overlay, normalize_field_text


ROOT = Path(__file__).resolve().parent
TEMPLATE_PDF_PATH = ROOT / "i-9.pdf"
LAYOUT_PATH = ROOT / "data" / "i9_field_layout.json"
DATA_PATH = ROOT / "data" / "data.json"

OUTPUT_DIR = ROOT / "output"
OVERLAY_PDF_PATH = OUTPUT_DIR / "i9_overlay.pdf"
PDF_PATH = OUTPUT_DIR / "report.pdf"
PREVIEW_PNG_STEM = "report"
PAGE_DATA_PATH = OUTPUT_DIR / "report_page_data.json"
BINDINGS_PATH = OUTPUT_DIR / "template_bindings.json"
COMPOSE_REPORT_PATH = OUTPUT_DIR / "compose_report.json"
RUN_REPORT_PATH = OUTPUT_DIR / "run_report.json"
FIELD_FIT_REPORT_PATH = OUTPUT_DIR / "field_fit_validation.json"
JIT_PATH = OUTPUT_DIR / "report.jit.jsonl"
PERF_PATH = OUTPUT_DIR / "report.perf.jsonl"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "component_mount_validation.json"
CSS_LAYER_REPORT_PATH = OUTPUT_DIR / "css_layers.json"
TEMPLATE_ASSET_REPORT_PATH = OUTPUT_DIR / "template_asset_validation.json"

CSS_LAYER_ORDER = [
    "styles/tokens.css",
    "components/styles/i9_overlay.css",
    "styles/report.css",
]

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
    "table-column",
    "table-column-group",
    "ruby",
    "ruby-base",
    "ruby-text",
    "ruby-base-container",
    "ruby-text-container",
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


def load_layout_and_values() -> tuple[dict[str, Any], dict[str, Any]]:
    if not LAYOUT_PATH.exists() or not DATA_PATH.exists():
        raise FileNotFoundError(
            "I-9 layout/data JSON not found. Run: python tools/build_i9_fields.py"
        )
    layout_payload = json.loads(LAYOUT_PATH.read_text(encoding="utf-8"))
    data_payload = json.loads(DATA_PATH.read_text(encoding="utf-8"))

    layout = layout_payload if isinstance(layout_payload, dict) else {}
    values_container = data_payload if isinstance(data_payload, dict) else {}
    values = values_container.get("values") if isinstance(values_container.get("values"), dict) else {}

    field_count = len(layout.get("fields") or [])
    if field_count == 0:
        raise ValueError(f"layout contains no fields: {LAYOUT_PATH}")
    if len(values) != field_count:
        raise ValueError(
            f"value count mismatch: values={len(values)} fields={field_count}; regenerate data JSON"
        )

    return layout, values


def _selector_scope_ok(selector: str) -> bool:
    cleaned = selector.strip()
    if not cleaned:
        return True
    if cleaned.startswith("@"):
        return True
    if cleaned in {":root", "html", "body"}:
        return True
    if cleaned.startswith("html ") or cleaned.startswith("body "):
        return True
    if '[data-fb-role="document-root"]' in cleaned:
        return True
    if "[data-fb-role='document-root']" in cleaned:
        return True
    if ".fb-document-root" in cleaned:
        return True
    return False


def _find_unscoped_selectors(css_text: str) -> list[str]:
    findings: list[str] = []
    for raw in re.findall(r"([^{}]+)\{", css_text):
        header = raw.strip()
        if not header or header.startswith("@"):
            continue
        for selector in [part.strip() for part in header.split(",")]:
            if not selector:
                continue
            if _selector_scope_ok(selector):
                continue
            findings.append(selector)
            if len(findings) >= 20:
                return findings
    return findings


def _find_engine_no_effect_declarations(css_text: str) -> list[dict[str, str]]:
    findings: list[dict[str, str]] = []
    for match in re.finditer(r"([a-zA-Z-]+)\s*:\s*([^;{}]+)", css_text):
        prop = match.group(1).strip().lower()
        value = match.group(2).strip().lower()
        if prop in NO_EFFECT_PROPERTIES:
            findings.append({"property": prop, "value": value})
        elif prop == "display" and any(token in value for token in NORMALIZED_DISPLAY_VALUES):
            findings.append({"property": prop, "value": value})
        if len(findings) >= 20:
            break
    return findings


def load_css_layers() -> tuple[str, list[dict[str, object]], list[dict[str, str]], list[dict[str, str]]]:
    manifest: list[dict[str, object]] = []
    css_parts: list[str] = []
    unscoped: list[dict[str, str]] = []
    no_effect: list[dict[str, str]] = []

    for rel in CSS_LAYER_ORDER:
        path = ROOT / rel
        exists = path.exists()
        text = path.read_text(encoding="utf-8") if exists else ""
        byte_count = len(text.encode("utf-8")) if exists else 0
        manifest.append({"path": rel, "exists": exists, "bytes": byte_count})
        if not exists or not text.strip():
            continue

        css_parts.append(f"/* layer: {rel} */\n{text}")

        if rel.startswith("components/styles/"):
            for selector in _find_unscoped_selectors(text):
                unscoped.append({"layer": rel, "selector": selector})
            for finding in _find_engine_no_effect_declarations(text):
                no_effect.append({"layer": rel, **finding})

    return "\n\n".join(css_parts), manifest, unscoped, no_effect


def _template_asset_validation() -> dict[str, Any]:
    if not TEMPLATE_PDF_PATH.exists():
        raise FileNotFoundError(f"template PDF not found: {TEMPLATE_PDF_PATH}")

    asset = fullbleed.vendored_asset(str(TEMPLATE_PDF_PATH), "pdf", name="i9-template")
    info = asset.info()

    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(TEMPLATE_PDF_PATH), "pdf", name="i9-template")
    bundled = [item for item in bundle.assets_info() if item.get("kind") == "pdf"]

    ok = (
        info.get("kind") == "pdf"
        and int(info.get("page_count") or 0) >= 1
        and not bool(info.get("encrypted"))
        and len(bundled) == 1
    )

    result = {
        "schema": "fullbleed.i9.template_asset_validation.v1",
        "ok": bool(ok),
        "template_pdf": str(TEMPLATE_PDF_PATH),
        "vendored_asset": info,
        "bundle_assets": bundled,
    }
    TEMPLATE_ASSET_REPORT_PATH.write_text(json.dumps(result, indent=2), encoding="utf-8")
    if not ok:
        raise RuntimeError("template PDF asset validation failed")
    return result


def create_engine(
    *,
    template_binding: dict[str, Any],
    debug: bool | None = None,
    debug_out: str | None = None,
    jit_mode: str | None = None,
) -> fullbleed.PdfEngine:
    bundle = fullbleed.AssetBundle()

    # Vendored defaults from `fullbleed init`.
    bundle.add_file(str(ROOT / "vendor/css/bootstrap.min.css"), "css", name="bootstrap")
    if _env_truthy("FULLBLEED_I9_EMBED_INTER"):
        bundle.add_file(str(ROOT / "vendor/fonts/Inter-Variable.ttf"), "font")
    bundle.add_file(str(ROOT / "vendor/icons/bootstrap-icons.svg"), "svg", name="bootstrap-icons")
    # First-class template PDF asset registration.
    bundle.add_file(str(TEMPLATE_PDF_PATH), "pdf", name="i9-template")

    debug_enabled = _env_truthy("FULLBLEED_DEBUG") if debug is None else bool(debug)
    perf_enabled = _env_truthy("FULLBLEED_PERF")
    debug_target = debug_out if debug_out is not None else (str(JIT_PATH) if debug_enabled else None)

    engine = fullbleed.PdfEngine(
        page_width="612pt",
        page_height="792pt",
        margin="0pt",
        template_binding=template_binding,
        reuse_xobjects=True,
        svg_form_xobjects=True,
        unicode_support=True,
        shape_text=True,
        unicode_metrics=True,
        debug=debug_enabled,
        debug_out=debug_target,
        perf=perf_enabled,
        perf_out=str(PERF_PATH) if perf_enabled else None,
        jit_mode=jit_mode,
    )

    engine.register_bundle(bundle)
    return engine


@Document(page="LETTER", margin="0in", title="Form I-9 Canonical Overlay", bootstrap=False)
def App(props=None):
    payload = props or {}
    layout = payload.get("layout") or {}
    values = payload.get("values") or {}
    return I9Overlay(layout=layout, values=values)


def build_html(*, layout: dict[str, Any], values: dict[str, Any]) -> str:
    artifact = App({"layout": layout, "values": values})
    return compile_document(artifact)


def _render_pdf_previews(pdf_path: Path, out_dir: Path, *, stem: str, dpi: int) -> list[str]:
    if dpi <= 0:
        return []
    scale = dpi / 72.0
    matrix = fitz.Matrix(scale, scale)
    paths: list[str] = []
    doc = fitz.open(pdf_path)
    try:
        for i in range(doc.page_count):
            pix = doc[i].get_pixmap(matrix=matrix, alpha=False)
            path = out_dir / f"{stem}_page{i + 1}.png"
            pix.save(path)
            paths.append(str(path))
    finally:
        doc.close()
    return paths


def _validate_css_layers(
    *,
    unscoped_selectors: list[dict[str, str]],
    no_effect_declarations: list[dict[str, str]],
    strict_validate: bool,
) -> None:
    if unscoped_selectors:
        print(f"[warn] Found {len(unscoped_selectors)} unscoped selector(s) in component CSS.")
        for item in unscoped_selectors[:5]:
            print(f"[warn] {item['layer']}: {item['selector']}")
        if strict_validate:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and unscoped selectors were found.")
            raise SystemExit(2)

    if no_effect_declarations:
        print(f"[warn] Found {len(no_effect_declarations)} engine no-effect declaration(s) in component CSS.")
        for item in no_effect_declarations[:5]:
            print(f"[warn] {item['layer']}: {item['property']}: {item['value']}")
        if strict_validate:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and engine no-effect declarations were found.")
            raise SystemExit(2)


def _field_fit_validation(
    *,
    layout: dict[str, Any],
    values: dict[str, Any],
    pdf_path: Path,
) -> dict[str, Any]:
    def _chars_in_rect(page_obj: fitz.Page, rect: fitz.Rect) -> str:
        raw = page_obj.get_text("rawdict")
        rows: list[tuple[float, str]] = []
        for block in raw.get("blocks", []):
            if not isinstance(block, dict):
                continue
            for line in block.get("lines", []) or []:
                for span in line.get("spans", []) or []:
                    for ch in span.get("chars", []) or []:
                        c = str(ch.get("c", ""))
                        if not c.strip():
                            continue
                        bbox = ch.get("bbox")
                        if not isinstance(bbox, (list, tuple)) or len(bbox) != 4:
                            continue
                        crect = fitz.Rect(float(bbox[0]), float(bbox[1]), float(bbox[2]), float(bbox[3]))
                        if crect.intersects(rect):
                            rows.append((float(crect.x0), c))
        rows.sort(key=lambda item: item[0])
        return "".join(c for _x, c in rows)

    doc = fitz.open(pdf_path)
    try:
        text_fields_total = 0
        text_fields_matched = 0
        checked_total = 0
        checked_hits = 0
        false_check_hits = 0
        samples: list[dict[str, Any]] = []

        fields = layout.get("fields") or []
        for field in fields:
            try:
                page_number = int(field.get("page", 0))
            except (TypeError, ValueError):
                continue
            if page_number <= 0 or page_number > doc.page_count:
                continue

            field_type = str(field.get("field_type", "Text"))
            key = str(field.get("key", ""))
            value = values.get(key)
            page = doc[page_number - 1]
            target = fitz.Rect(
                float(field.get("x_pt", 0.0)),
                float(field.get("y_pt", 0.0)),
                float(field.get("x_pt", 0.0)) + float(field.get("width_pt", 0.0)),
                float(field.get("y_pt", 0.0)) + float(field.get("height_pt", 0.0)),
            )

            if field_type == "CheckBox":
                checked = bool(value)
                search_hits = page.search_for("X")
                hit = False
                for rect in search_hits:
                    if rect.intersects(target):
                        hit = True
                        break
                    expand = fitz.Rect(target)
                    expand.x0 -= 2.0
                    expand.y0 -= 2.0
                    expand.x1 += 2.0
                    expand.y1 += 2.0
                    if rect.intersects(expand):
                        hit = True
                        break
                if checked:
                    checked_total += 1
                    if hit:
                        checked_hits += 1
                elif hit:
                    false_check_hits += 1
                if len(samples) < 12:
                    samples.append(
                        {
                            "page": page_number,
                            "key": key,
                            "field_type": field_type,
                            "checked": checked,
                            "hit": hit,
                        }
                    )
                continue

            text = normalize_field_text(field, value).strip()
            if not text:
                continue
            text_fields_total += 1
            is_comb = bool(field.get("comb")) or bool(int(field.get("field_flags") or 0) & (1 << 24))

            hit = False
            if is_comb:
                observed = _chars_in_rect(page, target)
                expected = "".join(ch for ch in text if ch.strip())
                observed_compact = "".join(ch for ch in observed if ch.isalnum())
                hit = observed_compact.startswith(expected) or (expected in observed_compact)
            else:
                search_hits = page.search_for(text)
                for rect in search_hits:
                    if rect.intersects(target):
                        hit = True
                        break
                    expand = fitz.Rect(target)
                    expand.x0 -= 6.0
                    expand.y0 -= 6.0
                    expand.x1 += 6.0
                    expand.y1 += 6.0
                    if rect.intersects(expand):
                        hit = True
                        break
            if hit:
                text_fields_matched += 1

            if len(samples) < 12:
                samples.append(
                    {
                        "page": page_number,
                        "key": key,
                        "field_type": field_type,
                        "value": text,
                        "hit": hit,
                    }
                )

        text_ratio = (
            float(text_fields_matched) / float(text_fields_total)
            if text_fields_total > 0
            else 1.0
        )
        checked_ratio = (
            float(checked_hits) / float(checked_total)
            if checked_total > 0
            else 1.0
        )

        return {
            "schema": "fullbleed.form_i9_field_fit_validation.v1",
            "ok": text_ratio >= 0.98 and checked_ratio >= 1.0,
            "text_fields_total": text_fields_total,
            "text_fields_matched": text_fields_matched,
            "text_match_ratio": round(text_ratio, 4),
            "checked_total": checked_total,
            "checked_hits": checked_hits,
            "checked_match_ratio": round(checked_ratio, 4),
            "unchecked_false_hits": false_check_hits,
            "samples": samples,
        }
    finally:
        doc.close()


def _run_mount_validation(
    *,
    layout: dict[str, Any],
    values: dict[str, Any],
    css: str,
    template_binding: dict[str, Any],
    strict_validate: bool,
) -> None:
    with tempfile.NamedTemporaryFile(prefix="fullbleed_mount_", suffix=".jit.jsonl", delete=False) as tmp:
        mount_jit_path = Path(tmp.name)

    try:
        validation_engine = create_engine(
            template_binding=template_binding,
            debug=True,
            debug_out=str(mount_jit_path),
            jit_mode="plan",
        )
        mount_validation = validate_component_mount(
            engine=validation_engine,
            node_or_component=App,
            props={"layout": layout, "values": values},
            css=css,
            debug_log=str(mount_jit_path),
            title="form-i9 component mount smoke",
            fail_on_overflow=False,
            fail_on_css_warnings=False,
            fail_on_known_loss=strict_validate,
            fail_on_html_asset_warning=True,
        )
    finally:
        if mount_jit_path.exists():
            mount_jit_path.unlink(missing_ok=True)

    COMPONENT_VALIDATION_PATH.write_text(json.dumps(mount_validation, indent=2), encoding="utf-8")
    if not mount_validation.get("ok", False):
        print(f"[error] Component mount validation failed: {COMPONENT_VALIDATION_PATH}")
        raise SystemExit(2)

    warnings = mount_validation.get("warnings") or []
    if warnings:
        print(f"[warn] Component mount validation warnings: {len(warnings)}")
        blocking_warnings = [
            warning
            for warning in warnings
            if str(warning.get("code", "")).upper() != "OVERFLOW"
        ]
        if strict_validate and blocking_warnings:
            print("[error] FULLBLEED_VALIDATE_STRICT=1 and mount warnings were detected.")
            raise SystemExit(2)
    print(f"[ok] Component mount validation: {COMPONENT_VALIDATION_PATH}")


def _build_template_binding(layout: dict[str, Any]) -> dict[str, Any]:
    page_count = int(layout.get("page_count") or len(layout.get("pages") or []))
    by_feature: dict[str, str] = {}
    for page_no in range(1, page_count + 1):
        by_feature[f"i9_page_{page_no}"] = "i9-template"
    return {
        "default_template_id": "i9-template",
        "feature_prefix": "fb.feature.",
        "by_feature": by_feature,
    }


def _build_compose_plan(*, bindings: list[dict[str, Any]], template_page_count: int) -> list[tuple[str, int, int, float, float]]:
    plan: list[tuple[str, int, int, float, float]] = []
    for item in bindings:
        overlay_page = int(item.get("page_index", 0))
        template_id = str(item.get("template_id", "i9-template"))
        template_page = overlay_page
        if template_page_count > 0:
            template_page = min(template_page, template_page_count - 1)
        plan.append((template_id, template_page, overlay_page, 0.0, 0.0))
    return plan


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    strict_validate = _env_truthy("FULLBLEED_VALIDATE_STRICT")
    image_dpi = _env_int("FULLBLEED_IMAGE_DPI", 144)

    layout, values = load_layout_and_values()
    template_asset = _template_asset_validation()
    template_page_count = int(template_asset["vendored_asset"].get("page_count") or 0)

    template_binding = _build_template_binding(layout)

    html = build_html(layout=layout, values=values)
    css, css_layers, unscoped_selectors, no_effect_declarations = load_css_layers()
    CSS_LAYER_REPORT_PATH.write_text(
        json.dumps(
            {
                "layers": css_layers,
                "unscoped_selector_count": len(unscoped_selectors),
                "no_effect_declaration_count": len(no_effect_declarations),
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    _validate_css_layers(
        unscoped_selectors=unscoped_selectors,
        no_effect_declarations=no_effect_declarations,
        strict_validate=strict_validate,
    )
    _run_mount_validation(
        layout=layout,
        values=values,
        css=css,
        template_binding=template_binding,
        strict_validate=strict_validate,
    )

    engine = create_engine(template_binding=template_binding)
    overlay_bytes, page_data, bindings = engine.render_pdf_with_page_data_and_template_bindings(html, css)
    OVERLAY_PDF_PATH.write_bytes(overlay_bytes)

    if page_data is not None:
        PAGE_DATA_PATH.write_text(json.dumps(page_data, indent=2), encoding="utf-8")

    if not isinstance(bindings, list):
        raise RuntimeError("engine did not return template bindings; template_binding pipeline is required")
    BINDINGS_PATH.write_text(json.dumps(bindings, indent=2), encoding="utf-8")

    overlay_page_count = len(bindings)
    if overlay_page_count != int(layout.get("page_count") or overlay_page_count):
        raise RuntimeError(
            f"overlay page count mismatch: expected={layout.get('page_count')} got={overlay_page_count}"
        )

    plan = _build_compose_plan(bindings=bindings, template_page_count=template_page_count)
    compose_result = fullbleed.finalize_compose_pdf(
        [("i9-template", str(TEMPLATE_PDF_PATH))],
        plan,
        str(OVERLAY_PDF_PATH),
        str(PDF_PATH),
    )
    COMPOSE_REPORT_PATH.write_text(json.dumps(compose_result, indent=2), encoding="utf-8")

    fit_report = _field_fit_validation(layout=layout, values=values, pdf_path=PDF_PATH)
    FIELD_FIT_REPORT_PATH.write_text(json.dumps(fit_report, indent=2), encoding="utf-8")
    if not fit_report.get("ok", False):
        raise RuntimeError(
            f"field fit validation failed: {FIELD_FIT_REPORT_PATH} "
            f"(text_match_ratio={fit_report.get('text_match_ratio')})"
        )

    composed_preview_paths = _render_pdf_previews(
        PDF_PATH,
        OUTPUT_DIR,
        stem=PREVIEW_PNG_STEM,
        dpi=image_dpi,
    )

    report = {
        "schema": "fullbleed.form_i9_example_report.v1",
        "ok": True,
        "template_pdf": str(TEMPLATE_PDF_PATH),
        "layout_path": str(LAYOUT_PATH),
        "data_path": str(DATA_PATH),
        "overlay_pdf": str(OVERLAY_PDF_PATH),
        "composed_pdf": str(PDF_PATH),
        "page_count": overlay_page_count,
        "field_count": len(layout.get("fields") or []),
        "template_asset_report": str(TEMPLATE_ASSET_REPORT_PATH),
        "bindings_path": str(BINDINGS_PATH),
        "compose_report": compose_result,
        "field_fit_report": fit_report,
        "preview_pngs": composed_preview_paths,
    }
    RUN_REPORT_PATH.write_text(json.dumps(report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote overlay PDF: {OVERLAY_PDF_PATH} ({len(overlay_bytes)} bytes)")
    print(f"[ok] Wrote composed PDF: {PDF_PATH}")
    if composed_preview_paths:
        print(f"[ok] Preview PNG: {composed_preview_paths[0]}")
    print(f"[ok] Template bindings: {BINDINGS_PATH}")
    print(f"[ok] Compose report: {COMPOSE_REPORT_PATH}")
    print(f"[ok] Run report: {RUN_REPORT_PATH}")
    print(f"[ok] CSS layers: {CSS_LAYER_REPORT_PATH}")
    if page_data is not None:
        print(f"[ok] Page data: {PAGE_DATA_PATH}")
    if _env_truthy("FULLBLEED_DEBUG"):
        print(f"[ok] JIT trace: {JIT_PATH}")
    if _env_truthy("FULLBLEED_PERF"):
        print(f"[ok] Perf trace: {PERF_PATH}")


if __name__ == "__main__":
    main()
