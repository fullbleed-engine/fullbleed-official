#!/usr/bin/env python
"""Generate living docs HTML/CSS inputs for the font + Bootstrap atlas."""
from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from html import escape
from pathlib import Path
from typing import Dict, List


PROJECT_ROOT = Path(__file__).resolve().parents[1]
DATA_DIR = PROJECT_ROOT / "data"
TEMPLATE_DIR = PROJECT_ROOT / "templates"
BUILD_DIR = PROJECT_ROOT / "build"
SPECIMEN_BUILD_DIR = BUILD_DIR / "specimens"
VENDOR_DIR = PROJECT_ROOT / "vendor"
VENDOR_CSS = VENDOR_DIR / "css"
LANGUAGE_SAMPLE_PATH = DATA_DIR / "language_samples.json"

GROUP_ORDER = ["Noto Global", "Sans Serif", "Serif", "Monospace"]
LATIN_PROFILE = ["en", "es", "fr", "de", "pt", "vi", "tr", "pl", "emoji"]
MONO_PROFILE = ["en", "es", "de", "tr", "pl", "ru", "el", "code", "emoji"]
GLOBAL_PROFILE = ["en", "es", "fr", "de", "ru", "el", "ar", "he", "ja", "ko", "zh-hans", "th", "hi", "emoji"]


def _slugify(value: str) -> str:
    out = []
    for ch in value.lower():
        if ch.isalnum():
            out.append(ch)
        else:
            out.append("-")
    slug = "".join(out).strip("-")
    while "--" in slug:
        slug = slug.replace("--", "-")
    return slug


def _load_json(path: Path) -> Dict:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def load_font_catalog() -> List[Dict]:
    data = _load_json(DATA_DIR / "font_catalog.json")
    fonts = data.get("fonts", [])
    if not isinstance(fonts, list):
        raise ValueError("data/font_catalog.json is missing 'fonts' list")
    return fonts


def load_feature_groups() -> List[Dict]:
    data = _load_json(DATA_DIR / "bootstrap_features.json")
    groups = data.get("groups", [])
    if not isinstance(groups, list):
        raise ValueError("data/bootstrap_features.json is missing 'groups' list")
    return groups


def load_language_samples() -> Dict[str, Dict]:
    data = _load_json(LANGUAGE_SAMPLE_PATH)
    rows = data.get("samples", [])
    if not isinstance(rows, list):
        raise ValueError("data/language_samples.json is missing 'samples' list")

    catalog: Dict[str, Dict] = {}
    for row in rows:
        if not isinstance(row, dict):
            continue
        sample_id = str(row.get("id", "")).strip().lower()
        if sample_id:
            catalog[sample_id] = row
    if not catalog:
        raise ValueError("data/language_samples.json did not produce any usable sample rows")
    return catalog


def _font_generic_fallback(font: Dict) -> str:
    group = font.get("group")
    if group == "Monospace":
        return "monospace"
    if group == "Serif":
        return "serif"
    return "sans-serif"


def _font_fallback_stack(font: Dict) -> str:
    package = font.get("package", "")
    group = font.get("group")
    if package == "noto-color-emoji":
        return '"Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif'
    if group == "Monospace":
        return '"Noto Sans Mono", "JetBrains Mono", Consolas, "Liberation Mono", monospace'
    if group == "Serif":
        return '"Noto Serif", Georgia, "Times New Roman", serif'
    return '"Noto Sans", "Segoe UI", "Helvetica Neue", Arial, sans-serif'


def _enrich_font_status(fonts: List[Dict]) -> List[Dict]:
    enriched = []
    for font in fonts:
        row = dict(font)
        filename = row["filename"]
        rel_path = Path("vendor") / "fonts" / filename
        abs_path = PROJECT_ROOT / rel_path
        row["asset_path"] = str(rel_path).replace("\\", "/")
        row["installed"] = abs_path.exists()
        row["font_family"] = f'FB {row["display_name"]}'
        row["class_name"] = f'font-family-{_slugify(row["package"])}'
        row["slug"] = _slugify(row["package"])
        row["specimen_html"] = f'specimens/{row["slug"]}.html'
        row["specimen_css"] = f'specimens/{row["slug"]}.css'
        row["specimen_summary"] = f'specimens/{row["slug"]}.summary.json'
        enriched.append(row)
    return enriched


def _font_language_profile(font: Dict) -> List[str]:
    package = str(font.get("package", ""))
    if package == "noto-color-emoji":
        return ["emoji", "en", "ja", "ko", "zh-hans", "ar", "he"]
    if package == "noto-sans-arabic":
        return ["ar", "en", "fr", "de", "emoji"]
    if package == "noto-sans-hebrew":
        return ["he", "en", "de", "fr", "emoji"]
    if package == "noto-sans-jp":
        return ["ja", "en", "zh-hans", "ko", "emoji"]
    if package == "noto-sans-kr":
        return ["ko", "en", "ja", "zh-hans", "emoji"]
    if package == "noto-sans-sc":
        return ["zh-hans", "en", "ja", "ko", "emoji"]
    if package == "noto-sans-thai":
        return ["th", "en", "vi", "emoji"]
    if package in {"noto-sans-regular", "noto-sans-italic", "noto-serif-regular", "noto-serif-italic"}:
        return GLOBAL_PROFILE
    if str(font.get("group", "")) == "Monospace" or package.endswith("mono"):
        return MONO_PROFILE
    return LATIN_PROFILE


def _language_rows_for_font(font: Dict, language_catalog: Dict[str, Dict]) -> List[Dict]:
    rows: List[Dict] = []
    seen_ids = set()
    for sample_id in _font_language_profile(font):
        if sample_id in seen_ids:
            continue
        sample = language_catalog.get(sample_id)
        if not sample:
            continue
        seen_ids.add(sample_id)
        text = str(sample.get("text", "")).replace("{font}", str(font.get("display_name", "Unknown")))
        rows.append(
            {
                "id": sample_id,
                "label": str(sample.get("label", sample_id)),
                "script": str(sample.get("script", "Unknown")),
                "lang": str(sample.get("lang", sample_id)),
                "dir": str(sample.get("dir", "ltr")),
                "text": text,
            }
        )
    return rows


def _primary_sample_text(language_rows: List[Dict], fallback: str) -> str:
    for row in language_rows:
        if row.get("id") != "code":
            return str(row.get("text", fallback))
    return fallback


def _feature_chips(groups: List[Dict]) -> str:
    chips: List[str] = []
    for group in groups:
        name = escape(str(group.get("name", "Unknown")))
        items = group.get("items", [])
        for item in items:
            chips.append(
                f'<span class="badge rounded-pill text-bg-light border feature-chip" '
                f'data-feature-group="{name}">{name}: {escape(str(item))}</span>'
            )
    return "\n".join(chips)


def _font_sections(fonts: List[Dict]) -> str:
    grouped: Dict[str, List[Dict]] = {}
    for font in fonts:
        grouped.setdefault(font.get("group", "Other"), []).append(font)

    chunks: List[str] = []
    for group in GROUP_ORDER + sorted(set(grouped.keys()) - set(GROUP_ORDER)):
        rows = grouped.get(group)
        if not rows:
            continue
        cards: List[str] = []
        for font in rows:
            status = "Installed" if font["installed"] else "Missing"
            status_class = "text-bg-success" if font["installed"] else "text-bg-secondary"
            cards.append(
                (
                    '<div class="col">\n'
                    '  <article class="card h-100 border-0 shadow-sm font-card">\n'
                    '    <div class="card-body">\n'
                    f'      <div class="d-flex justify-content-between align-items-start mb-2">\n'
                    f'        <h3 class="h6 mb-0">{escape(font["display_name"])}</h3>\n'
                    f'        <span class="badge {status_class}">{status}</span>\n'
                    "      </div>\n"
                    f'      <p class="small text-muted mb-2"><code>{escape(font["package"])}</code></p>\n'
                    f'      <p class="font-sample {font["class_name"]}">{escape(font["atlas_sample_text"])}</p>\n'
                    '      <dl class="row small mb-0">\n'
                    '        <dt class="col-4 text-muted">File</dt>\n'
                    f'        <dd class="col-8"><code>{escape(font["filename"])}</code></dd>\n'
                    '        <dt class="col-4 text-muted">License</dt>\n'
                    f'        <dd class="col-8"><code>{escape(font["license"])}</code></dd>\n'
                    "      </dl>\n"
                    f'      <a class="btn btn-sm btn-outline-primary mt-3" href="{escape(font["specimen_html"])}">'
                    f'This is {escape(font["display_name"])}</a>\n'
                    "    </div>\n"
                    "  </article>\n"
                    "</div>"
                )
            )

        chunks.append(
            (
                '<section class="font-group mb-5">\n'
                f'  <h3 class="h5 mb-3">{escape(group)}</h3>\n'
                '  <div class="row row-cols-1 row-cols-md-2 row-cols-xl-3 g-3">\n'
                + "\n".join(cards)
                + "\n  </div>\n"
                "</section>"
            )
        )

    return "\n".join(chunks)


def _font_registry_rows(fonts: List[Dict]) -> str:
    rows: List[str] = []
    for font in fonts:
        status = "Installed" if font["installed"] else "Missing"
        status_class = "text-bg-success" if font["installed"] else "text-bg-secondary"
        usage_bundle = f'bundle.add_file {font["filename"]} kind=font name={font["package"]}'
        usage_class = f'css.class {font["class_name"]}'
        usage_family = f'css.family {font["font_family"]}'
        rows.append(
            (
                "<tr>\n"
                f'  <td><strong>{escape(font["display_name"])}</strong><div class="small text-muted">'
                f'<code>{escape(font["group"])}</code></div><div class="small">'
                f'<a href="{escape(font["specimen_html"])}">specimen</a></div></td>\n'
                f'  <td><code>{escape(font["package"])}</code></td>\n'
                f'  <td class="usage-cell"><div class="usage-line"><code>{escape(usage_bundle)}</code></div>'
                f'<div class="usage-line"><code>{escape(usage_class)}</code></div>'
                f'<div class="usage-line"><code>{escape(usage_family)}</code></div></td>\n'
                f'  <td><code>{escape(font["license"])}</code></td>\n'
                f'  <td><code>{escape(font["filename"])}</code></td>\n'
                f'  <td><span class="badge {status_class}">{status}</span></td>\n'
                "</tr>"
            )
        )
    return "\n".join(rows)


def _font_css(fonts: List[Dict], font_url_prefix: str) -> Dict[str, str]:
    face_rules: List[str] = []
    class_rules: List[str] = []
    for font in fonts:
        family = font["font_family"]
        filename = font["filename"]
        class_name = font["class_name"]
        fallback = _font_fallback_stack(font)
        style = "italic" if "italic" in font["package"] else "normal"
        if font["installed"]:
            face_rules.append(
                (
                    "@font-face {\n"
                    f'  font-family: "{family}";\n'
                    f'  src: url("{font_url_prefix}/{filename}") format("truetype");\n'
                    f"  font-style: {style};\n"
                    "  font-weight: 100 900;\n"
                    "  font-display: swap;\n"
                    "}\n"
                )
            )
        class_rules.append(
            (
                f".{class_name} {{\n"
                f'  font-family: "{family}", {fallback};\n'
                "}\n"
            )
        )

    return {
        "font_face_rules": "\n".join(face_rules).strip(),
        "font_class_rules": "\n".join(class_rules).strip(),
    }


def _render_template(text: str, replacements: Dict[str, str]) -> str:
    output = text
    for key, value in replacements.items():
        output = output.replace("{{" + key + "}}", value)
    return output


def _specimen_language_cards(language_rows: List[Dict]) -> str:
    cards: List[str] = []
    for row in language_rows:
        is_code = row["id"] == "code"
        sample_copy = f'<code>{escape(row["text"])}</code>' if is_code else escape(row["text"])
        sample_class = "sample-line font-target sample-code" if is_code else "sample-line font-target"
        cards.append(
            (
                '<article class="col-md-6">\n'
                '  <section class="card h-100 border-0 shadow-sm sample-card">\n'
                '    <div class="card-body">\n'
                f'      <p class="small text-uppercase fw-semibold text-muted mb-2">{escape(row["label"])} | '
                f'{escape(row["script"])}</p>\n'
                f'      <p class="{sample_class} mb-0" lang="{escape(row["lang"])}" '
                f'dir="{escape(row["dir"])}">{sample_copy}</p>\n'
                "    </div>\n"
                "  </section>\n"
                "</article>"
            )
        )
    return "\n".join(cards)


def _font_face_rule(font: Dict, font_url_prefix: str) -> str:
    if not font["installed"]:
        return "/* font asset missing; using fallback stack only */"
    style = "italic" if "italic" in font["package"] else "normal"
    return (
        "@font-face {\n"
        f'  font-family: "{font["font_family"]}";\n'
        f'  src: url("{font_url_prefix}/{font["filename"]}") format("truetype");\n'
        f"  font-style: {style};\n"
        "  font-weight: 100 900;\n"
        "  font-display: swap;\n"
        "}\n"
    )


def _render_specimen_index(items: List[Dict], generated_at: str) -> str:
    rows: List[str] = []
    for item in items:
        status = "Installed" if item.get("installed") else "Missing"
        status_class = "text-bg-success" if item.get("installed") else "text-bg-secondary"
        rows.append(
            (
                '<li class="list-group-item d-flex justify-content-between align-items-center">\n'
                f'  <a href="{escape(item["html"])}" class="text-decoration-none">{escape(item["display_name"])}</a>\n'
                f'  <span class="badge {status_class}">{status}</span>\n'
                "</li>"
            )
        )
    return (
        "<!DOCTYPE html>\n"
        '<html lang="en">\n'
        "<head>\n"
        '  <meta charset="utf-8">\n'
        '  <meta name="viewport" content="width=device-width, initial-scale=1">\n'
        "  <title>Living Docs Font Specimens</title>\n"
        "</head>\n"
        '<body style="font-family: Segoe UI, Arial, sans-serif; padding: 1rem;">\n'
        "  <main>\n"
        "    <h1>Fullbleed Font Specimen Index</h1>\n"
        f"    <p>Generated: {escape(generated_at)}</p>\n"
        f'    <p>Total specimen pages: {len(items)}</p>\n'
        '    <ul class="list-group" style="list-style: none; padding-left: 0;">\n'
        + "\n".join(rows)
        + "\n    </ul>\n"
        "  </main>\n"
        "</body>\n"
        "</html>\n"
    )


def _build_font_specimens(
    fonts: List[Dict],
    generated_at: str,
    font_url_prefix: str,
    write_files: bool,
) -> Dict:
    template_html = (TEMPLATE_DIR / "font_specimen.template.html").read_text(encoding="utf-8")
    template_css = (TEMPLATE_DIR / "font_specimen.template.css").read_text(encoding="utf-8")

    items: List[Dict] = []
    if write_files:
        SPECIMEN_BUILD_DIR.mkdir(parents=True, exist_ok=True)

    for font in fonts:
        language_rows = font.get("language_rows", [])
        html_rel = Path("build") / font["specimen_html"]
        css_rel = Path("build") / font["specimen_css"]
        summary_rel = Path("build") / font["specimen_summary"]
        html_path = PROJECT_ROOT / html_rel
        css_path = PROJECT_ROOT / css_rel
        summary_path = PROJECT_ROOT / summary_rel

        specimen_summary = {
            "schema": "fullbleed.font_specimen.v1",
            "generated_at": generated_at,
            "package": font["package"],
            "display_name": font["display_name"],
            "group": font["group"],
            "installed": font["installed"],
            "filename": font["filename"],
            "license": font["license"],
            "language_ids": [row["id"] for row in language_rows],
            "language_count": len(language_rows),
            "html": str(html_rel).replace("\\", "/"),
            "css": str(css_rel).replace("\\", "/"),
        }

        html_out = _render_template(
            template_html,
            {
                "FONT_NAME": escape(font["display_name"]),
                "FONT_PACKAGE": escape(font["package"]),
                "FONT_GROUP": escape(font["group"]),
                "FONT_FILE": escape(font["filename"]),
                "FONT_LICENSE": escape(font["license"]),
                "GENERATED_AT": escape(generated_at),
                "FONT_CSS_FILE": escape(f"{font['slug']}.css"),
                "FONT_STATUS": "Installed" if font["installed"] else "Missing",
                "FONT_STATUS_BADGE": "text-bg-success" if font["installed"] else "text-bg-secondary",
                "LANGUAGE_SPECIMENS": _specimen_language_cards(language_rows),
                "MACHINE_SUMMARY_JSON": escape(json.dumps(specimen_summary, ensure_ascii=True, indent=2)),
            },
        )

        css_out = _render_template(
            template_css,
            {
                "FONT_FACE_RULE": _font_face_rule(font, font_url_prefix),
                "FONT_FAMILY": font["font_family"],
                "FONT_FALLBACK_STACK": _font_fallback_stack(font),
                "FONT_GENERIC_FALLBACK": _font_generic_fallback(font),
            },
        )

        if write_files:
            html_path.parent.mkdir(parents=True, exist_ok=True)
            html_path.write_text(html_out, encoding="utf-8")
            css_path.write_text(css_out, encoding="utf-8")
            summary_path.write_text(json.dumps(specimen_summary, ensure_ascii=True, indent=2), encoding="utf-8")

        items.append(
            {
                "package": font["package"],
                "display_name": font["display_name"],
                "group": font["group"],
                "installed": font["installed"],
                "language_ids": [row["id"] for row in language_rows],
                "html": str(Path(font["slug"] + ".html")).replace("\\", "/"),
                "css": str(Path(font["slug"] + ".css")).replace("\\", "/"),
                "summary": str(Path(font["slug"] + ".summary.json")).replace("\\", "/"),
            }
        )

    manifest = {
        "schema": "fullbleed.font_specimens_manifest.v1",
        "generated_at": generated_at,
        "count": len(items),
        "items": items,
    }

    index_path = SPECIMEN_BUILD_DIR / "index.html"
    manifest_path = SPECIMEN_BUILD_DIR / "specimens.manifest.json"
    index_html = _render_specimen_index(items, generated_at)

    if write_files:
        index_path.write_text(index_html, encoding="utf-8")
        manifest_path.write_text(json.dumps(manifest, ensure_ascii=True, indent=2), encoding="utf-8")

    return {
        "manifest": manifest,
        "paths": {
            "index": str(index_path),
            "manifest": str(manifest_path),
        },
    }


def build_inputs(
    font_url_prefix: str = "../vendor/fonts",
    specimen_font_url_prefix: str = "../../vendor/fonts",
    write_files: bool = True,
    build_specimens: bool = True,
) -> Dict:
    fonts = _enrich_font_status(load_font_catalog())
    groups = load_feature_groups()
    language_catalog = load_language_samples()

    for font in fonts:
        language_rows = _language_rows_for_font(font, language_catalog)
        font["language_rows"] = language_rows
        font["atlas_sample_text"] = _primary_sample_text(language_rows, font.get("sample_text", ""))

    total = len(fonts)
    installed = sum(1 for font in fonts if font["installed"])
    missing = total - installed
    language_ids = sorted({row["id"] for font in fonts for row in font.get("language_rows", [])})

    css_parts = _font_css(fonts, font_url_prefix=font_url_prefix)
    feature_html = _feature_chips(groups)
    font_sections = _font_sections(fonts)
    font_registry_rows = _font_registry_rows(fonts)

    bootstrap_candidates = [
        VENDOR_CSS / "bootstrap.min.css",
        VENDOR_DIR / "bootstrap.min.css",
    ]

    generated_at = datetime.now(timezone.utc).isoformat()

    specimen_result = {
        "manifest": {"count": 0, "items": []},
        "paths": {"index": None, "manifest": None},
    }
    if build_specimens:
        specimen_result = _build_font_specimens(
            fonts=fonts,
            generated_at=generated_at,
            font_url_prefix=specimen_font_url_prefix,
            write_files=write_files,
        )

    summary = {
        "schema": "fullbleed.living_docs_summary.v1",
        "generated_at": generated_at,
        "font_total": total,
        "font_installed": installed,
        "font_missing": missing,
        "bootstrap_css_installed": any(path.exists() for path in bootstrap_candidates),
        "installed_font_files": [font["asset_path"] for font in fonts if font["installed"]],
        "missing_packages": [font["package"] for font in fonts if not font["installed"]],
        "language_sample_ids": language_ids,
        "language_sample_count": len(language_ids),
        "font_specimen_count": specimen_result["manifest"]["count"],
        "font_registry_rows": len(fonts),
        "font_specimen_manifest": (
            str(Path(specimen_result["paths"]["manifest"]).relative_to(PROJECT_ROOT)).replace("\\", "/")
            if specimen_result["paths"]["manifest"]
            else None
        ),
    }

    template_html = (TEMPLATE_DIR / "atlas.template.html").read_text(encoding="utf-8")
    template_css = (TEMPLATE_DIR / "atlas.template.css").read_text(encoding="utf-8")

    html_out = _render_template(
        template_html,
        {
            "GENERATED_AT": summary["generated_at"],
            "FONT_TOTAL": str(total),
            "FONT_INSTALLED": str(installed),
            "FONT_MISSING": str(missing),
            "FEATURE_CHIPS": feature_html,
            "FONT_GROUP_SECTIONS": font_sections,
            "FONT_REGISTRY_ROWS": font_registry_rows,
            "MACHINE_SUMMARY_JSON": escape(json.dumps(summary, ensure_ascii=True, indent=2)),
        },
    )
    css_out = _render_template(
        template_css,
        {
            "FONT_FACE_RULES": css_parts["font_face_rules"],
            "FONT_CLASS_RULES": css_parts["font_class_rules"],
        },
    )

    html_path = BUILD_DIR / "atlas.html"
    css_path = BUILD_DIR / "atlas.css"
    summary_path = BUILD_DIR / "atlas.summary.json"

    if write_files:
        BUILD_DIR.mkdir(parents=True, exist_ok=True)
        html_path.write_text(html_out, encoding="utf-8")
        css_path.write_text(css_out, encoding="utf-8")
        summary_path.write_text(json.dumps(summary, ensure_ascii=True, indent=2), encoding="utf-8")

    return {
        "html": html_out,
        "css": css_out,
        "summary": summary,
        "specimens": specimen_result["manifest"],
        "paths": {
            "html": str(html_path),
            "css": str(css_path),
            "summary": str(summary_path),
            "specimens_index": specimen_result["paths"]["index"],
            "specimens_manifest": specimen_result["paths"]["manifest"],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Build living docs HTML/CSS inputs")
    parser.add_argument(
        "--font-url-prefix",
        default="../vendor/fonts",
        help="URL prefix used for @font-face sources in generated CSS.",
    )
    parser.add_argument(
        "--specimen-font-url-prefix",
        default="../../vendor/fonts",
        help="URL prefix used for @font-face sources in generated specimen CSS.",
    )
    parser.add_argument(
        "--skip-font-specimens",
        action="store_true",
        help="Do not write per-font specimen HTML/CSS artifacts.",
    )
    parser.add_argument("--no-write", action="store_true", help="Do not write files, print summary only.")
    args = parser.parse_args()

    result = build_inputs(
        font_url_prefix=args.font_url_prefix,
        specimen_font_url_prefix=args.specimen_font_url_prefix,
        write_files=not args.no_write,
        build_specimens=not args.skip_font_specimens,
    )
    print(json.dumps(result["summary"], ensure_ascii=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
