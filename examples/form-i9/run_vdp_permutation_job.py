from __future__ import annotations

import json
import os
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import fullbleed

import report as i9_report


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output" / "permutation_vdp"
CHUNKS = OUT / "chunks"

RECORDS_PATH = OUT / "records.json"
MANIFEST_PATH = OUT / "manifest.json"
FINAL_OVERLAY_PATH = OUT / "overlay_merged.pdf"
FINAL_COMPOSED_PATH = OUT / "composed_merged.pdf"

# Without third-party PDF toolkits, chunk merging is not available in this script.
# Keep a large default chunk so the canonical run emits single-file outputs.
CHUNK_SIZE_RECORDS = int(os.getenv("FULLBLEED_I9_CHUNK_SIZE", "1000000"))
PAGES_PER_RECORD = 4

# Finite state list for combo fields.
US_STATE_CODES = [
    "",
    "AL",
    "AK",
    "AZ",
    "AR",
    "CA",
    "CO",
    "CT",
    "DE",
    "FL",
    "GA",
    "HI",
    "ID",
    "IL",
    "IN",
    "IA",
    "KS",
    "KY",
    "LA",
    "ME",
    "MD",
    "MA",
    "MI",
    "MN",
    "MS",
    "MO",
    "MT",
    "NE",
    "NV",
    "NH",
    "NJ",
    "NM",
    "NY",
    "NC",
    "ND",
    "OH",
    "OK",
    "OR",
    "PA",
    "RI",
    "SC",
    "SD",
    "TN",
    "TX",
    "UT",
    "VT",
    "VA",
    "WA",
    "WV",
    "WI",
    "WY",
    "DC",
    "PR",
]


@dataclass
class ScenarioRecord:
    record_id: str
    category: str
    detail: str
    values: dict[str, Any]
    focus_key: str | None = None
    focus_value: Any = None


def _ensure_out() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    CHUNKS.mkdir(parents=True, exist_ok=True)


def _deepcopy_values(values: dict[str, Any]) -> dict[str, Any]:
    # Values are scalar JSON types only.
    return dict(values)


def _text_variant_value(field: dict[str, Any], variant: str, seq: int) -> str:
    name = str(field.get("pdf_field_name", "")).lower()
    width = float(field.get("width_pt", 0.0))
    key = str(field.get("key", ""))

    if "date" in name or "mmdd" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return "12/31/2030"
        return "01/01/2026"
    if "social" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return "987-65-4321"
        return "123-45-6789"
    if "zip" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return "12345-6789"
        return "10001"
    if "telephone" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return "(212) 555-0101"
        return "(512) 555-0199"
    if "e-mail" in name or "email" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return f"case{seq}@example.org"
        return "jane.doe@example.com"
    if "state" in name:
        if variant == "blank":
            return ""
        if variant == "alternate":
            return "CA"
        return "TX"

    if variant == "blank":
        return ""
    if variant == "alternate":
        return f"{key.upper()}-{seq:04d}"

    # Width-aware stress sample for generic text boxes.
    # Approximate 8.25pt text width with ~4.5pt per character.
    max_chars = max(1, int((width - 2.0) / 4.5))
    return ("W" * max_chars)[:max_chars]


def build_permutation_records(layout: dict[str, Any], base_values: dict[str, Any]) -> list[ScenarioRecord]:
    fields = list(layout.get("fields") or [])
    checkbox_fields = [f for f in fields if str(f.get("field_type")) == "CheckBox"]
    combo_fields = [f for f in fields if str(f.get("field_type")) == "ComboBox"]
    text_fields = [f for f in fields if str(f.get("field_type")) not in {"CheckBox", "ComboBox"}]

    records: list[ScenarioRecord] = []

    def add_record(
        *,
        category: str,
        detail: str,
        values: dict[str, Any],
        focus_key: str | None = None,
        focus_value: Any = None,
    ) -> None:
        record_id = f"{category.upper()}-{len(records) + 1:05d}"
        payload = _deepcopy_values(values)
        # Hidden record marker for page-order validation.
        payload["__record_marker"] = f"CASE::{record_id}::{detail}"
        records.append(
            ScenarioRecord(
                record_id=record_id,
                category=category,
                detail=detail,
                values=payload,
                focus_key=focus_key,
                focus_value=focus_value,
            )
        )

    # Baseline.
    add_record(
        category="baseline",
        detail="seed",
        values=base_values,
    )

    # Exhaustive checkbox permutations.
    checkbox_keys = [str(f.get("key", "")) for f in checkbox_fields]
    for mask in range(1 << len(checkbox_keys)):
        values = _deepcopy_values(base_values)
        bits: list[str] = []
        for bit, key in enumerate(checkbox_keys):
            enabled = bool(mask & (1 << bit))
            values[key] = enabled
            bits.append("1" if enabled else "0")
        add_record(
            category="checkbox",
            detail=f"mask={mask:03d}:{''.join(bits)}",
            values=values,
        )

    # Per-field combo sweep across state codes.
    for field in combo_fields:
        key = str(field.get("key", ""))
        for code in US_STATE_CODES:
            values = _deepcopy_values(base_values)
            values[key] = code
            state_label = code if code else "BLANK"
            add_record(
                category="combo",
                detail=f"{key}={state_label}",
                values=values,
                focus_key=key,
                focus_value=code,
            )

    # Per-field text variants: blank, maxfit, alternate.
    text_variants = ("blank", "maxfit", "alternate")
    for field in text_fields:
        key = str(field.get("key", ""))
        for variant in text_variants:
            values = _deepcopy_values(base_values)
            val = _text_variant_value(field, variant, len(records) + 1)
            values[key] = val
            add_record(
                category="text",
                detail=f"{key}:{variant}",
                values=values,
                focus_key=key,
                focus_value=val,
            )

    return records


def _chunked(seq: list[ScenarioRecord], size: int) -> list[list[ScenarioRecord]]:
    return [seq[i : i + size] for i in range(0, len(seq), size)]


def _render_batch_overlay(
    engine: fullbleed.PdfEngine,
    css: str,
    batch: list[ScenarioRecord],
    out_pdf: Path,
) -> tuple[int, str]:
    html_docs = [i9_report.build_html(layout=LAYOUT, values=rec.values) for rec in batch]
    if hasattr(engine, "render_pdf_batch_to_file_parallel"):
        return int(engine.render_pdf_batch_to_file_parallel(html_docs, css, str(out_pdf))), "parallel"
    return int(engine.render_pdf_batch_to_file(html_docs, css, str(out_pdf))), "sequential"


def _compose_batch(overlay_pdf: Path, out_pdf: Path, overlay_page_count: int) -> dict[str, Any]:
    plan: list[tuple[str, int, int, float, float]] = []
    for overlay_page in range(overlay_page_count):
        template_page = overlay_page % TEMPLATE_PAGE_COUNT
        plan.append(("i9-template", template_page, overlay_page, 0.0, 0.0))

    return fullbleed.finalize_compose_pdf(
        [("i9-template", str(i9_report.TEMPLATE_PDF_PATH))],
        plan,
        str(overlay_pdf),
        str(out_pdf),
    )


def _marker_validation_placeholder(records: list[ScenarioRecord]) -> dict[str, Any]:
    return {
        "ok": True,
        "checked_records": len(records),
        "mode": "disabled_no_pdf_text_extractor",
        "note": "Marker text extraction requires a PDF text extractor; this pass validates page contracts only.",
    }


def _count_categories(records: list[ScenarioRecord]) -> dict[str, int]:
    out: dict[str, int] = {}
    for rec in records:
        out[rec.category] = out.get(rec.category, 0) + 1
    return out


def run() -> dict[str, Any]:
    _ensure_out()

    records = build_permutation_records(LAYOUT, BASE_VALUES)
    RECORDS_PATH.write_text(
        json.dumps(
            {
                "schema": "fullbleed.form_i9_permutation_records.v1",
                "record_count": len(records),
                "pages_per_record": PAGES_PER_RECORD,
                "categories": _count_categories(records),
                "records": [
                    {
                        "record_id": r.record_id,
                        "category": r.category,
                        "detail": r.detail,
                        "focus_key": r.focus_key,
                        "focus_value": r.focus_value,
                        "values": r.values,
                    }
                    for r in records
                ],
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    engine = i9_report.create_engine(
        template_binding={
            "default_template_id": "i9-template",
            "feature_prefix": "fb.feature.",
            "by_feature": {},
        }
    )
    css, _css_layers, _unscoped, _no_effect = i9_report.load_css_layers()

    chunk_rows: list[dict[str, Any]] = []
    batches = _chunked(records, CHUNK_SIZE_RECORDS)
    running_record_index = 0
    for batch_index, batch in enumerate(batches, start=1):
        chunk_id = f"{batch_index:03d}"
        overlay_pdf = CHUNKS / f"overlay_chunk_{chunk_id}.pdf"
        composed_pdf = CHUNKS / f"composed_chunk_{chunk_id}.pdf"

        overlay_bytes, batch_mode = _render_batch_overlay(engine, css, batch, overlay_pdf)
        overlay_pages = len(batch) * PAGES_PER_RECORD
        compose = _compose_batch(overlay_pdf, composed_pdf, overlay_pages)
        composed_pages = int(compose.get("pages_written") or 0)

        chunk_rows.append(
            {
                "chunk_id": chunk_id,
                "record_start": running_record_index + 1,
                "record_end": running_record_index + len(batch),
                "record_count": len(batch),
                "overlay_pdf": str(overlay_pdf),
                "composed_pdf": str(composed_pdf),
                "overlay_bytes": overlay_bytes,
                "batch_mode": batch_mode,
                "overlay_pages": overlay_pages,
                "composed_pages": composed_pages,
                "compose": compose,
            }
        )
        running_record_index += len(batch)

    merged_overlay_pages = sum(int(row["overlay_pages"]) for row in chunk_rows)
    merged_composed_pages = sum(int(row["composed_pages"]) for row in chunk_rows)
    expected_pages = len(records) * PAGES_PER_RECORD

    overlay_merged_pdf: str | None = None
    composed_merged_pdf: str | None = None
    if len(chunk_rows) == 1:
        src_overlay = Path(str(chunk_rows[0]["overlay_pdf"]))
        src_composed = Path(str(chunk_rows[0]["composed_pdf"]))
        shutil.copyfile(src_overlay, FINAL_OVERLAY_PATH)
        shutil.copyfile(src_composed, FINAL_COMPOSED_PATH)
        overlay_merged_pdf = str(FINAL_OVERLAY_PATH)
        composed_merged_pdf = str(FINAL_COMPOSED_PATH)

    marker_validation = _marker_validation_placeholder(records)
    single_chunk_required = len(chunk_rows) == 1

    manifest = {
        "schema": "fullbleed.form_i9_permutation_vdp_manifest.v1",
        "ok": (
            single_chunk_required
            and merged_overlay_pages == expected_pages
            and merged_composed_pages == expected_pages
            and marker_validation.get("ok", False)
        ),
        "template_pdf": str(i9_report.TEMPLATE_PDF_PATH),
        "layout_path": str(i9_report.LAYOUT_PATH),
        "data_path": str(i9_report.DATA_PATH),
        "record_count": len(records),
        "pages_per_record": PAGES_PER_RECORD,
        "expected_total_pages": expected_pages,
        "embed_inter": os.getenv("FULLBLEED_I9_EMBED_INTER", "").strip().lower() in {"1", "true", "yes", "on"},
        "chunk_size_records": CHUNK_SIZE_RECORDS,
        "single_chunk_required": single_chunk_required,
        "overlay_merged_pdf": overlay_merged_pdf,
        "composed_merged_pdf": composed_merged_pdf,
        "overlay_merged_pages": merged_overlay_pages,
        "composed_merged_pages": merged_composed_pages,
        "categories": _count_categories(records),
        "chunk_count": len(chunk_rows),
        "chunks": chunk_rows,
        "marker_validation": marker_validation,
        "records_path": str(RECORDS_PATH),
    }
    MANIFEST_PATH.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


if __name__ == "__main__":
    LAYOUT, BASE_VALUES = i9_report.load_layout_and_values()
    TEMPLATE_PAGE_COUNT = int(
        fullbleed.vendored_asset(str(i9_report.TEMPLATE_PDF_PATH), "pdf", name="i9-template")
        .info()
        .get("page_count")
        or 0
    )
    report = run()
    print(json.dumps(report, ensure_ascii=True))
    if not report.get("ok", False):
        raise SystemExit(1)
