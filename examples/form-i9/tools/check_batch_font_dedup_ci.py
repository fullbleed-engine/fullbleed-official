from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

import fullbleed


ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from components.fb_ui import Document, compile_document  # noqa: E402
from components.i9_overlay import I9Overlay  # noqa: E402


TEMPLATE_PDF_PATH = ROOT / "i-9.pdf"
LAYOUT_PATH = ROOT / "data" / "i9_field_layout.json"
DATA_PATH = ROOT / "data" / "data.json"

OUT_DIR = ROOT / "output" / "ci_font_dedup"
REPORT_PATH = OUT_DIR / "report.json"

CSS_LAYER_ORDER = [
    "styles/tokens.css",
    "components/styles/i9_overlay.css",
    "styles/report.css",
]

RECORD_COUNT = max(2, int(os.getenv("FULLBLEED_I9_CI_RECORDS", "12")))


@Document(page="LETTER", margin="0in", title="I-9 CI Font Dedup Probe", bootstrap=False)
def App(props=None):
    payload = props or {}
    layout = payload.get("layout") or {}
    values = payload.get("values") or {}
    return I9Overlay(layout=layout, values=values)


def load_layout_and_values() -> tuple[dict[str, Any], dict[str, Any]]:
    if not LAYOUT_PATH.exists() or not DATA_PATH.exists():
        raise FileNotFoundError(
            "I-9 layout/data JSON not found. Run: python examples/form-i9/tools/build_i9_fields.py"
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


def load_css() -> str:
    css_parts: list[str] = []
    for rel in CSS_LAYER_ORDER:
        path = ROOT / rel
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        if text.strip():
            css_parts.append(f"/* layer: {rel} */\n{text}")
    # Keep CI deterministic: force the embedded Inter path in author CSS.
    css_parts.append(
        '[data-fb-role="document-root"] { font-family: "Inter", "Helvetica Neue", Arial, sans-serif; }'
    )
    return "\n\n".join(css_parts)


def build_html(*, layout: dict[str, Any], values: dict[str, Any]) -> str:
    artifact = App({"layout": layout, "values": values})
    return compile_document(artifact)


def make_record_values(base_values: dict[str, Any], index: int) -> dict[str, Any]:
    values = dict(base_values)
    # Marker text varies per record so batch pages are not byte-identical clones.
    values["p01_additional_information"] = f"CI_FONT_DEDUP_RECORD_{index:03d}"
    # Toggle a few booleans to vary checkboxes as well.
    values["p01_i_am_a_citizen_of_the_u"] = (index % 2) == 0
    values["p01_i_am_a_noncitizen_nationa"] = (index % 3) == 0
    return values


def build_template_binding(layout: dict[str, Any]) -> dict[str, Any]:
    page_count = int(layout.get("page_count") or len(layout.get("pages") or []))
    by_feature: dict[str, str] = {}
    for page_no in range(1, page_count + 1):
        by_feature[f"i9_page_{page_no}"] = "i9-template"
    return {
        "default_template_id": "i9-template",
        "feature_prefix": "fb.feature.",
        "by_feature": by_feature,
    }


def create_engine(layout: dict[str, Any]) -> fullbleed.PdfEngine:
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(ROOT / "vendor/css/bootstrap.min.css"), "css", name="bootstrap")
    bundle.add_file(str(ROOT / "vendor/fonts/Inter-Variable.ttf"), "font")
    bundle.add_file(str(ROOT / "vendor/icons/bootstrap-icons.svg"), "svg", name="bootstrap-icons")
    bundle.add_file(str(TEMPLATE_PDF_PATH), "pdf", name="i9-template")

    engine = fullbleed.PdfEngine(
        page_width="612pt",
        page_height="792pt",
        margin="0pt",
        template_binding=build_template_binding(layout),
        reuse_xobjects=True,
        svg_form_xobjects=True,
        unicode_support=True,
        shape_text=True,
        unicode_metrics=True,
    )
    engine.register_bundle(bundle)
    return engine


def count_pdf_token(path: Path, pattern: bytes) -> int:
    blob = path.read_bytes()
    return len(re.findall(re.escape(pattern), blob))


def run_probe(
    engine: fullbleed.PdfEngine,
    css: str,
    html_docs: list[str],
    *,
    mode: str,
    out_pdf: Path,
) -> dict[str, Any]:
    if mode == "parallel":
        method = getattr(engine, "render_pdf_batch_to_file_parallel", None)
    elif mode == "sequential":
        method = getattr(engine, "render_pdf_batch_to_file", None)
    else:
        raise ValueError(f"unsupported mode: {mode}")

    if method is None:
        return {"mode": mode, "skipped": False, "ok": False, "reason": "method_missing"}

    bytes_written = int(method(html_docs, css, str(out_pdf)))
    font_file2_count = count_pdf_token(out_pdf, b"/FontFile2")
    type0_count = count_pdf_token(out_pdf, b"/Subtype /Type0")
    cid_type2_count = count_pdf_token(out_pdf, b"/Subtype /CIDFontType2")

    ok = font_file2_count == 1 and type0_count == 1 and cid_type2_count == 1
    return {
        "mode": mode,
        "skipped": False,
        "ok": ok,
        "pdf": str(out_pdf),
        "bytes_written": bytes_written,
        "file_size": out_pdf.stat().st_size,
        "font_file2_count": font_file2_count,
        "type0_count": type0_count,
        "cidfont_type2_count": cid_type2_count,
    }


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    layout, base_values = load_layout_and_values()
    css = load_css()
    engine = create_engine(layout)

    html_docs = [
        build_html(layout=layout, values=make_record_values(base_values, i))
        for i in range(RECORD_COUNT)
    ]

    runs: list[dict[str, Any]] = []
    runs.append(
        run_probe(
            engine,
            css,
            html_docs,
            mode="sequential",
            out_pdf=OUT_DIR / "overlay_batch_sequential.pdf",
        )
    )
    runs.append(
        run_probe(
            engine,
            css,
            html_docs,
            mode="parallel",
            out_pdf=OUT_DIR / "overlay_batch_parallel.pdf",
        )
    )

    checked_runs = [r for r in runs if not r.get("skipped", False)]
    ok = bool(checked_runs) and all(bool(r.get("ok", False)) for r in checked_runs)
    report = {
        "schema": "fullbleed.form_i9_ci_font_dedup.v1",
        "ok": ok,
        "record_count": RECORD_COUNT,
        "runs": runs,
    }
    REPORT_PATH.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))

    if not ok:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
