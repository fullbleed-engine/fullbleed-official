#!/usr/bin/env python
"""Run CSS parity fixtures with stage-oriented assertions."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import tempfile
import time
import zlib
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple


try:
    import fullbleed
except Exception as exc:  # pragma: no cover - import guard for CI/runtime only
    print(
        json.dumps(
            {
                "schema": "fullbleed.css_fixture_suite.v1",
                "ok": False,
                "code": "IMPORT_ERROR",
                "message": f"failed to import fullbleed: {exc}",
            },
            ensure_ascii=True,
        )
    )
    raise SystemExit(1)


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _stable_json_hash(payload: Dict[str, Any] | List[Any]) -> str:
    normalized = json.dumps(
        payload,
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


def _artifact_hash(pdf_bytes: bytes, image_bytes: List[bytes]) -> str:
    return _stable_json_hash(
        {
            "schema": "fullbleed.css_fixture_artifact_digest.v1",
            "pdf_sha256": _sha256_bytes(pdf_bytes),
            "image_sha256": [_sha256_bytes(img) for img in image_bytes],
        }
    )


def _sanitize_fixture_id(value: str) -> str:
    cleaned = "".join(ch if ch.isalnum() or ch in ("-", "_", ".") else "_" for ch in value)
    return cleaned or "fixture"


def _write_fixture_artifacts(
    artifacts_dir: Path,
    fixture_id: str,
    pdf_bytes: bytes,
    image_bytes: List[bytes],
    result_payload: Dict[str, Any],
) -> Dict[str, Any]:
    fixture_dir = artifacts_dir / _sanitize_fixture_id(fixture_id)
    fixture_dir.mkdir(parents=True, exist_ok=True)

    output_pdf = fixture_dir / "output.pdf"
    output_pdf.write_bytes(pdf_bytes)

    image_paths: List[str] = []
    for idx, image in enumerate(image_bytes, start=1):
        page_path = fixture_dir / f"output_page{idx}.png"
        page_path.write_bytes(image)
        image_paths.append(str(page_path).replace("\\", "/"))

    hashes_path = fixture_dir / "hashes.json"
    hashes_path.write_text(
        json.dumps(
            {
                "schema": "fullbleed.css_fixture_artifacts_hashes.v1",
                "pdf_sha256": _sha256_bytes(pdf_bytes),
                "image_sha256": [_sha256_bytes(img) for img in image_bytes],
                "artifact_sha256": _artifact_hash(pdf_bytes, image_bytes),
            },
            ensure_ascii=True,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    render_result_path = fixture_dir / "render_result.json"
    render_result_path.write_text(
        json.dumps(result_payload, ensure_ascii=True, indent=2) + "\n",
        encoding="utf-8",
    )

    return {
        "dir": str(fixture_dir).replace("\\", "/"),
        "pdf": str(output_pdf).replace("\\", "/"),
        "images": image_paths,
        "hashes": str(hashes_path).replace("\\", "/"),
        "render_result": str(render_result_path).replace("\\", "/"),
    }


def _paeth_predictor(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa = abs(p - a)
    pb = abs(p - b)
    pc = abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def _decode_png_rgba_rows(data: bytes) -> Tuple[int, int, List[bytes]]:
    if len(data) < 8 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG byte stream")

    width: Optional[int] = None
    height: Optional[int] = None
    bit_depth: Optional[int] = None
    color_type: Optional[int] = None
    idat = bytearray()

    pos = 8
    while pos + 8 <= len(data):
        length = int.from_bytes(data[pos : pos + 4], "big")
        ctype = data[pos + 4 : pos + 8]
        pos += 8
        chunk = data[pos : pos + length]
        pos += length
        pos += 4  # CRC
        if ctype == b"IHDR":
            width = int.from_bytes(chunk[0:4], "big")
            height = int.from_bytes(chunk[4:8], "big")
            bit_depth = chunk[8]
            color_type = chunk[9]
        elif ctype == b"IDAT":
            idat.extend(chunk)
        elif ctype == b"IEND":
            break

    if width is None or height is None or bit_depth is None or color_type is None:
        raise ValueError("invalid PNG structure")
    if bit_depth != 8:
        raise ValueError(f"unsupported bit depth: {bit_depth}")
    if color_type == 2:
        bpp = 3
    elif color_type == 6:
        bpp = 4
    else:
        raise ValueError(f"unsupported color type: {color_type}")

    raw = zlib.decompress(bytes(idat))
    stride = width * bpp
    expected = (stride + 1) * height
    if len(raw) < expected:
        raise ValueError("truncated PNG payload")

    prev = bytearray(stride)
    off = 0
    rows: List[bytes] = []
    for row_idx in range(height):
        filter_type = raw[off]
        off += 1
        row = bytearray(raw[off : off + stride])
        off += stride

        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + left) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                up = prev[i]
                row[i] = (row[i] + ((left + up) // 2)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                up = prev[i]
                up_left = prev[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + _paeth_predictor(left, up, up_left)) & 0xFF
        else:
            raise ValueError(f"unsupported PNG filter type: {filter_type}")

        if bpp == 3:
            rgba = bytearray(width * 4)
            for col in range(width):
                src = col * 3
                dst = col * 4
                rgba[dst] = row[src]
                rgba[dst + 1] = row[src + 1]
                rgba[dst + 2] = row[src + 2]
                rgba[dst + 3] = 255
            rows.append(bytes(rgba))
        else:
            rows.append(bytes(row))

        prev = row
    return width, height, rows


def _rgba_pixel(rows: List[bytes], x: int, y: int) -> Tuple[int, int, int, int]:
    row = rows[y]
    idx = x * 4
    return row[idx], row[idx + 1], row[idx + 2], row[idx + 3]


def _read_png_pixel(data: bytes, x: int, y: int) -> Tuple[int, int, int, int]:
    width, height, rows = _decode_png_rgba_rows(data)
    if x < 0 or y < 0 or x >= width or y >= height:
        raise ValueError(f"pixel out of bounds ({x}, {y}) for PNG {width}x{height}")
    return _rgba_pixel(rows, x, y)


def _within_tolerance(got: Iterable[int], exp: Iterable[int], tolerance: int) -> bool:
    return all(abs(int(g) - int(e)) <= tolerance for g, e in zip(got, exp))


def _evaluate_layout_assertions(
    image_bytes: List[bytes], expected_assertions: List[Any]
) -> Tuple[bool, List[str], Dict[str, Any]]:
    failures: List[str] = []
    checks: List[Dict[str, Any]] = []
    page_cache: Dict[int, Tuple[int, int, List[bytes]]] = {}

    def _get_page(page: int) -> Tuple[int, int, List[bytes]]:
        if page not in page_cache:
            if page < 1 or page > len(image_bytes):
                raise ValueError(
                    f"page {page} out of bounds for {len(image_bytes)} rendered page(s)"
                )
            page_cache[page] = _decode_png_rgba_rows(image_bytes[page - 1])
        return page_cache[page]

    for idx, raw_assertion in enumerate(expected_assertions):
        if not isinstance(raw_assertion, dict):
            failures.append(f"layout_assertion_invalid:{idx}")
            checks.append(
                {
                    "index": idx,
                    "ok": False,
                    "reason": "assertion_must_be_object",
                }
            )
            continue

        kind = str(raw_assertion.get("kind", "color_run_length")).strip().lower()
        if kind != "color_run_length":
            failures.append(f"layout_assertion_invalid_kind:{idx}:{kind}")
            checks.append(
                {
                    "index": idx,
                    "ok": False,
                    "kind": kind,
                    "reason": "unsupported_kind",
                }
            )
            continue

        try:
            page = int(raw_assertion["page"])
            axis = str(raw_assertion.get("axis", "x")).strip().lower()
            if axis not in {"x", "y"}:
                raise ValueError(f"invalid axis: {axis}")
            fixed = int(raw_assertion["fixed"])
            start = int(raw_assertion.get("start", 0))
            expected_rgba = [int(v) for v in _safe_list(raw_assertion.get("rgba"))]
            if len(expected_rgba) != 4:
                raise ValueError("rgba must have 4 integer entries")
            tolerance = int(raw_assertion.get("tolerance", 0))
            width, height, rows = _get_page(page)

            axis_limit = width if axis == "x" else height
            cross_limit = height if axis == "x" else width
            if fixed < 0 or fixed >= cross_limit:
                raise ValueError(
                    f"fixed coordinate {fixed} out of bounds for axis {axis} and page {width}x{height}"
                )
            if start < 0 or start >= axis_limit:
                raise ValueError(
                    f"start coordinate {start} out of bounds for axis {axis} and page {width}x{height}"
                )

            run_length = 0
            pos = start
            while pos < axis_limit:
                if axis == "x":
                    pixel = _rgba_pixel(rows, pos, fixed)
                else:
                    pixel = _rgba_pixel(rows, fixed, pos)
                if not _within_tolerance(pixel, expected_rgba, tolerance):
                    break
                run_length += 1
                pos += 1

            expected_length_raw = raw_assertion.get("expected_length")
            if expected_length_raw is not None:
                expected_length = int(expected_length_raw)
                length_tolerance = int(raw_assertion.get("length_tolerance", 0))
                ok = abs(run_length - expected_length) <= length_tolerance
                expected_meta: Dict[str, Any] = {
                    "expected_length": expected_length,
                    "length_tolerance": length_tolerance,
                }
            else:
                min_length = int(raw_assertion.get("min_length", 0))
                max_length_raw = raw_assertion.get("max_length")
                max_length = axis_limit if max_length_raw is None else int(max_length_raw)
                ok = min_length <= run_length <= max_length
                expected_meta = {"min_length": min_length, "max_length": max_length}

            checks.append(
                {
                    "index": idx,
                    "ok": ok,
                    "kind": kind,
                    "page": page,
                    "axis": axis,
                    "fixed": fixed,
                    "start": start,
                    "expected_rgba": expected_rgba,
                    "tolerance": tolerance,
                    "actual_run_length": run_length,
                    **expected_meta,
                }
            )
            if not ok:
                failures.append(f"layout_assertion_failed:{idx}")
        except Exception as exc:  # noqa: BLE001
            failures.append(f"layout_assertion_error:{idx}:{exc}")
            checks.append(
                {
                    "index": idx,
                    "ok": False,
                    "kind": kind,
                    "reason": str(exc),
                }
            )

    return (
        not failures,
        failures,
        {
            "assertion_count": len(expected_assertions),
            "checks": checks,
        },
    )


@dataclass
class FixtureFile:
    path: Path
    payload: Dict[str, Any]

    @property
    def fixture_id(self) -> str:
        return str(self.payload["id"])

    @property
    def labels(self) -> List[str]:
        labels = self.payload.get("labels")
        if not isinstance(labels, list):
            return ["full"]
        out = [str(v).strip() for v in labels if str(v).strip()]
        return out or ["full"]


def _load_fixture_files(fixtures_dir: Path) -> List[FixtureFile]:
    files = sorted(fixtures_dir.glob("*.json"))
    out: List[FixtureFile] = []
    for path in files:
        payload = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            raise ValueError(f"fixture must be an object: {path}")
        fixture_id = payload.get("id")
        if not fixture_id:
            raise ValueError(f"fixture is missing id: {path}")
        out.append(FixtureFile(path=path, payload=payload))
    if not out:
        raise ValueError(f"no fixtures found in {fixtures_dir}")
    return out


def _fixture_selection(
    fixtures: List[FixtureFile],
    selected_ids: Optional[List[str]],
    selected_labels: Optional[List[str]],
) -> List[FixtureFile]:
    selected = fixtures
    if selected_labels:
        wanted_labels = {label.strip() for label in selected_labels if label.strip()}
        selected = [
            fixture
            for fixture in selected
            if wanted_labels.intersection(set(fixture.labels))
        ]

    if selected_ids:
        wanted_ids = set(selected_ids)
        selected = [f for f in selected if f.fixture_id in wanted_ids]
        missing = sorted(wanted_ids.difference({f.fixture_id for f in selected}))
        if missing:
            raise ValueError(f"unknown fixture ids: {', '.join(missing)}")
    return selected


def _safe_list(value: Any) -> List[Any]:
    return value if isinstance(value, list) else []


def _safe_dict(value: Any) -> Dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _parse_debug_log(log_path: Path) -> Dict[str, Any]:
    if not log_path.exists():
        return {"events": [], "summary_counts_total": {}, "summary_contexts": []}

    events: List[Dict[str, Any]] = []
    summary_counts_total: Dict[str, int] = {}
    summary_contexts: List[Dict[str, Any]] = []
    for raw in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        raw = raw.strip()
        if not raw:
            continue
        try:
            obj = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if not isinstance(obj, dict):
            continue
        events.append(obj)
        if obj.get("type") != "debug.summary":
            continue
        counts = obj.get("counts")
        if not isinstance(counts, dict):
            continue
        summary_contexts.append(
            {
                "context": obj.get("context"),
                "counts": counts,
            }
        )
        for key, value in counts.items():
            try:
                count = int(value)
            except Exception:  # noqa: BLE001
                continue
            summary_counts_total[key] = summary_counts_total.get(key, 0) + count
    return {
        "events": events,
        "summary_counts_total": summary_counts_total,
        "summary_contexts": summary_contexts,
    }


def _event_matches(event: Dict[str, Any], condition: Dict[str, Any]) -> bool:
    for key, expected in condition.items():
        if event.get(key) != expected:
            return False
    return True


def _evaluate_diagnostics(
    parsed_debug: Dict[str, Any], expected_diagnostics: Dict[str, Any]
) -> Tuple[bool, List[str], Dict[str, Any]]:
    failures: List[str] = []
    checks: List[Dict[str, Any]] = []
    summary_counts = _safe_dict(parsed_debug.get("summary_counts_total"))
    events = _safe_list(parsed_debug.get("events"))

    for cond in _safe_list(expected_diagnostics.get("event_contains")):
        if not isinstance(cond, dict):
            failures.append("diagnostic_event_contains_invalid")
            continue
        matched = any(_event_matches(event, cond) for event in events if isinstance(event, dict))
        checks.append({"kind": "event_contains", "condition": cond, "ok": matched})
        if not matched:
            failures.append(f"diagnostic_event_missing:{json.dumps(cond, ensure_ascii=True)}")

    for key, expected in _safe_dict(expected_diagnostics.get("summary_count_at_least")).items():
        try:
            expected_num = int(expected)
        except Exception:  # noqa: BLE001
            failures.append(f"diagnostic_count_invalid:{key}")
            continue
        got = int(summary_counts.get(str(key), 0))
        ok = got >= expected_num
        checks.append(
            {
                "kind": "summary_count_at_least",
                "key": str(key),
                "expected": expected_num,
                "actual": got,
                "ok": ok,
            }
        )
        if not ok:
            failures.append(f"diagnostic_count_low:{key}:expected>={expected_num}:got={got}")

    for key, expected in _safe_dict(expected_diagnostics.get("summary_count_equals")).items():
        try:
            expected_num = int(expected)
        except Exception:  # noqa: BLE001
            failures.append(f"diagnostic_count_invalid:{key}")
            continue
        got = int(summary_counts.get(str(key), 0))
        ok = got == expected_num
        checks.append(
            {
                "kind": "summary_count_equals",
                "key": str(key),
                "expected": expected_num,
                "actual": got,
                "ok": ok,
            }
        )
        if not ok:
            failures.append(f"diagnostic_count_mismatch:{key}:expected={expected_num}:got={got}")

    ok = not failures
    return (
        ok,
        failures,
        {
            "checks": checks,
            "summary_counts_total": summary_counts,
            "summary_context_count": len(_safe_list(parsed_debug.get("summary_contexts"))),
        },
    )


def _computed_event_matches(event: Dict[str, Any], assertion: Dict[str, Any]) -> bool:
    node = str(event.get("node", ""))
    node_exact = assertion.get("node")
    node_contains = assertion.get("node_contains")
    if node_exact is not None and node != str(node_exact):
        return False
    if node_contains is not None and str(node_contains) not in node:
        return False
    return True


def _evaluate_compute_assertions(
    parsed_debug: Dict[str, Any], expected_assertions: List[Any]
) -> Tuple[bool, List[str], Dict[str, Any]]:
    failures: List[str] = []
    checks: List[Dict[str, Any]] = []
    computed_events = [
        event
        for event in _safe_list(parsed_debug.get("events"))
        if isinstance(event, dict) and event.get("type") == "css.computed"
    ]

    for idx, raw_assertion in enumerate(expected_assertions):
        if not isinstance(raw_assertion, dict):
            failures.append(f"compute_assertion_invalid:{idx}")
            checks.append(
                {
                    "index": idx,
                    "ok": False,
                    "reason": "assertion_must_be_object",
                }
            )
            continue

        candidates = [
            event for event in computed_events if _computed_event_matches(event, raw_assertion)
        ]
        pick_raw = raw_assertion.get("pick", 0)
        try:
            pick = int(pick_raw)
        except Exception:  # noqa: BLE001
            pick = 0

        if pick < 0 or pick >= len(candidates):
            failures.append(f"compute_assertion_node_missing:{idx}")
            checks.append(
                {
                    "index": idx,
                    "ok": False,
                    "reason": "node_not_found",
                    "node": raw_assertion.get("node"),
                    "node_contains": raw_assertion.get("node_contains"),
                    "candidate_count": len(candidates),
                    "pick": pick,
                }
            )
            continue

        event = candidates[pick]
        style_actual = _safe_dict(event.get("style"))
        style_expected = _safe_dict(raw_assertion.get("style"))
        style_mismatches: List[Dict[str, Any]] = []
        for key, expected_value in style_expected.items():
            actual_value = style_actual.get(str(key))
            if actual_value != expected_value:
                style_mismatches.append(
                    {
                        "key": str(key),
                        "expected": expected_value,
                        "actual": actual_value,
                    }
                )

        vars_actual = _safe_list(event.get("vars_unresolved"))
        vars_expected = raw_assertion.get("vars_unresolved")
        vars_mismatch = None
        if isinstance(vars_expected, list):
            if vars_actual != vars_expected:
                vars_mismatch = {"expected": vars_expected, "actual": vars_actual}

        vars_count_expected = raw_assertion.get("vars_unresolved_count")
        vars_count_mismatch = None
        if vars_count_expected is not None:
            try:
                expected_count = int(vars_count_expected)
            except Exception:  # noqa: BLE001
                expected_count = None
            if expected_count is None or len(vars_actual) != expected_count:
                vars_count_mismatch = {
                    "expected": vars_count_expected,
                    "actual": len(vars_actual),
                }

        ok = (
            not style_mismatches
            and vars_mismatch is None
            and vars_count_mismatch is None
        )
        checks.append(
            {
                "index": idx,
                "ok": ok,
                "node": event.get("node"),
                "style_mismatches": style_mismatches,
                "vars_mismatch": vars_mismatch,
                "vars_count_mismatch": vars_count_mismatch,
            }
        )
        if not ok:
            failures.append(f"compute_assertion_failed:{idx}")

    return (
        not failures,
        failures,
        {
            "assertion_count": len(expected_assertions),
            "computed_event_count": len(computed_events),
            "checks": checks,
        },
    )


def _run_fixture(
    fixture: FixtureFile, update_stability: bool, artifacts_dir: Optional[Path]
) -> Dict[str, Any]:
    payload = fixture.payload
    html = str(payload.get("html", ""))
    css = str(payload.get("css", ""))
    if not html.strip():
        raise ValueError(f"{fixture.fixture_id}: html is empty")
    if not css.strip():
        raise ValueError(f"{fixture.fixture_id}: css is empty")

    dpi = int(payload.get("dpi", 120))
    expected = payload.get("expected", {}) if isinstance(payload.get("expected"), dict) else {}
    expected_page_count = expected.get("page_count")
    paint_samples = _safe_list(expected.get("paint_samples"))
    expected_compute_assertions = _safe_list(expected.get("compute_assertions"))
    expected_layout_assertions = _safe_list(expected.get("layout_assertions"))
    expected_warnings = [str(v) for v in _safe_list(payload.get("expected_warnings"))]
    stability_expected = payload.get("stability_hash")
    expected_diagnostics = _safe_dict(payload.get("expected_diagnostics"))
    run_debug = (
        bool(payload.get("debug", False))
        or bool(expected_diagnostics)
        or bool(expected_compute_assertions)
    )
    debug_log_path: Optional[Path] = None
    if run_debug:
        debug_log_path = Path(tempfile.gettempdir()) / (
            f"fullbleed_css_fixture_{fixture.fixture_id}_{os.getpid()}_{time.time_ns()}.jsonl"
        )

    if run_debug and debug_log_path is not None:
        engine = fullbleed.PdfEngine(debug=True, debug_out=str(debug_log_path))
    else:
        engine = fullbleed.PdfEngine()

    parser_ok = True
    parser_error = None
    pdf_bytes: bytes = b""
    try:
        pdf_bytes = bytes(engine.render_pdf(html, css))
    except Exception as exc:  # noqa: BLE001
        parser_ok = False
        parser_error = str(exc)

    compute_ok = False
    layout_ok = False
    paint_ok = False
    warnings: List[str] = []
    glyph_report: List[Dict[str, Any]] = []
    image_bytes: List[bytes] = []
    page_data: Optional[Dict[str, Any]] = None
    paint_checks: List[Dict[str, Any]] = []
    diagnostics_ok = True
    diagnostics_details: Dict[str, Any] = {}
    compute_assertions_ok = True
    compute_assertions_details: Dict[str, Any] = {
        "assertion_count": len(expected_compute_assertions),
        "checks": [],
    }
    layout_assertions_ok = not bool(expected_layout_assertions)
    layout_assertions_details: Dict[str, Any] = {
        "assertion_count": len(expected_layout_assertions),
        "checks": [],
    }

    if parser_ok:
        try:
            pdf2, page_data_obj, glyph_obj = engine.render_pdf_with_page_data_and_glyph_report(
                html, css
            )
            pdf_bytes = bytes(pdf2)
            page_data = page_data_obj if isinstance(page_data_obj, dict) else None
            glyph_report = list(glyph_obj) if isinstance(glyph_obj, list) else []
            if glyph_report:
                warnings.append("missing_glyphs")
            compute_ok = True
        except Exception as exc:  # noqa: BLE001
            warnings.append(f"compute_error:{exc}")

    if compute_ok:
        try:
            image_bytes = [bytes(b) for b in engine.render_image_pages(html, css, dpi)]
            page_count = len(image_bytes)
            if expected_page_count is None or int(expected_page_count) == page_count:
                layout_ok = True
            else:
                warnings.append(
                    f"layout_page_count_mismatch:expected={expected_page_count}:got={page_count}"
                )
        except Exception as exc:  # noqa: BLE001
            warnings.append(f"layout_error:{exc}")

    if layout_ok:
        paint_ok = True
        for check in paint_samples:
            try:
                page = int(check["page"])
                x = int(check["x"])
                y = int(check["y"])
                expected_rgba = [int(v) for v in check["rgba"]]
                tolerance = int(check.get("tolerance", 0))
                if page < 1 or page > len(image_bytes):
                    raise ValueError(
                        f"page {page} out of bounds for {len(image_bytes)} rendered page(s)"
                    )
                got = list(_read_png_pixel(image_bytes[page - 1], x, y))
                ok = _within_tolerance(got, expected_rgba, tolerance)
                paint_checks.append(
                    {
                        "page": page,
                        "x": x,
                        "y": y,
                        "expected_rgba": expected_rgba,
                        "actual_rgba": got,
                        "tolerance": tolerance,
                        "ok": ok,
                    }
                )
                if not ok:
                    paint_ok = False
                    warnings.append(f"paint_mismatch:page={page}:x={x}:y={y}")
            except Exception as exc:  # noqa: BLE001
                paint_ok = False
                warnings.append(f"paint_error:{exc}")

    if layout_ok and expected_layout_assertions:
        layout_assertions_ok, layout_failures, layout_result = _evaluate_layout_assertions(
            image_bytes, expected_layout_assertions
        )
        warnings.extend(layout_failures)
        layout_assertions_details = layout_result

    pdf_sha = _sha256_bytes(pdf_bytes) if pdf_bytes else None
    img_sha = [_sha256_bytes(img) for img in image_bytes]
    stability_actual = _artifact_hash(pdf_bytes, image_bytes) if pdf_bytes else None
    stability_ok = (
        True
        if (update_stability or not stability_expected or not stability_actual)
        else str(stability_expected) == str(stability_actual)
    )
    if (
        not update_stability
        and stability_expected
        and stability_actual
        and not stability_ok
    ):
        warnings.append("stability_hash_mismatch")

    if run_debug and debug_log_path is not None:
        parsed_debug = _parse_debug_log(debug_log_path)
        if expected_compute_assertions:
            compute_assertions_ok, compute_failures, compute_result = (
                _evaluate_compute_assertions(parsed_debug, expected_compute_assertions)
            )
            warnings.extend(compute_failures)
            compute_assertions_details = compute_result
        if expected_diagnostics:
            diagnostics_ok, diagnostic_failures, diagnostic_result = _evaluate_diagnostics(
                parsed_debug, expected_diagnostics
            )
            warnings.extend(diagnostic_failures)
            diagnostics_details = diagnostic_result
        else:
            diagnostics_details = {
                "summary_counts_total": parsed_debug.get("summary_counts_total", {}),
                "summary_context_count": len(_safe_list(parsed_debug.get("summary_contexts"))),
            }
        diagnostics_details["log_path"] = str(debug_log_path)

    warnings_ok = sorted(warnings) == sorted(expected_warnings)
    overall_ok = (
        parser_ok
        and compute_ok
        and compute_assertions_ok
        and layout_ok
        and layout_assertions_ok
        and paint_ok
        and diagnostics_ok
        and warnings_ok
        and stability_ok
    )

    emitted_artifacts: Optional[Dict[str, Any]] = None
    if artifacts_dir is not None and pdf_bytes:
        emitted_artifacts = _write_fixture_artifacts(
            artifacts_dir=artifacts_dir,
            fixture_id=fixture.fixture_id,
            pdf_bytes=pdf_bytes,
            image_bytes=image_bytes,
            result_payload={
                "schema": "fullbleed.css_fixture_render_result.v1",
                "id": fixture.fixture_id,
                "ok": overall_ok,
                "warnings_ok": warnings_ok,
                "expected_warnings": expected_warnings,
                "actual_warnings": warnings,
                "stages": {
                    "parser": {"ok": parser_ok, "error": parser_error},
                    "compute": {
                        "ok": compute_ok,
                        "glyph_report_count": len(glyph_report),
                        "page_data_present": page_data is not None,
                        "assertions_enabled": bool(expected_compute_assertions),
                        "assertions_ok": compute_assertions_ok,
                        "assertions": compute_assertions_details,
                    },
                    "layout": {
                        "ok": layout_ok,
                        "expected_page_count": expected_page_count,
                        "actual_page_count": len(image_bytes),
                        "dpi": dpi,
                        "assertions_enabled": bool(expected_layout_assertions),
                        "assertions_ok": layout_assertions_ok,
                        "assertions": layout_assertions_details,
                    },
                    "paint": {"ok": paint_ok, "checks": paint_checks},
                    "diagnostics": {
                        "ok": diagnostics_ok,
                        "enabled": run_debug,
                        "details": diagnostics_details,
                    },
                },
            },
        )

    return {
        "id": fixture.fixture_id,
        "path": str(fixture.path).replace("\\", "/"),
        "labels": fixture.labels,
        "description": payload.get("description", ""),
        "required_features": _safe_list(payload.get("required_features")),
        "expected_warnings": expected_warnings,
        "actual_warnings": warnings,
        "warnings_ok": warnings_ok,
        "stages": {
            "parser": {"ok": parser_ok, "error": parser_error},
            "compute": {
                "ok": compute_ok,
                "glyph_report_count": len(glyph_report),
                "page_data_present": page_data is not None,
                "assertions_enabled": bool(expected_compute_assertions),
                "assertions_ok": compute_assertions_ok,
                "assertions": compute_assertions_details,
            },
            "layout": {
                "ok": layout_ok,
                "expected_page_count": expected_page_count,
                "actual_page_count": len(image_bytes),
                "dpi": dpi,
                "assertions_enabled": bool(expected_layout_assertions),
                "assertions_ok": layout_assertions_ok,
                "assertions": layout_assertions_details,
            },
            "paint": {
                "ok": paint_ok,
                "checks": paint_checks,
            },
            "diagnostics": {
                "ok": diagnostics_ok,
                "enabled": run_debug,
                "details": diagnostics_details,
            },
        },
        "artifacts": {
            "pdf_bytes": len(pdf_bytes),
            "pdf_sha256": pdf_sha,
            "image_count": len(image_bytes),
            "image_sha256": img_sha,
            "stability_hash_expected": stability_expected,
            "stability_hash_actual": stability_actual,
            "stability_hash_ok": stability_ok,
            "emitted": emitted_artifacts,
        },
        "ok": overall_ok,
    }


def _update_stability_hashes(
    fixtures: List[FixtureFile], results: Dict[str, Dict[str, Any]]
) -> None:
    for fixture in fixtures:
        result = results.get(fixture.fixture_id)
        if not result:
            continue
        value = result["artifacts"].get("stability_hash_actual")
        if not value:
            continue
        fixture.payload["stability_hash"] = value
        fixture.path.write_text(
            json.dumps(fixture.payload, ensure_ascii=True, indent=2) + "\n",
            encoding="utf-8",
        )


def _fixture_exception_result(fixture: FixtureFile, error: Exception) -> Dict[str, Any]:
    message = str(error)
    return {
        "id": fixture.fixture_id,
        "path": str(fixture.path).replace("\\", "/"),
        "labels": fixture.labels,
        "description": fixture.payload.get("description", ""),
        "required_features": _safe_list(fixture.payload.get("required_features")),
        "expected_warnings": _safe_list(fixture.payload.get("expected_warnings")),
        "actual_warnings": [f"fixture_error:{message}"],
        "warnings_ok": False,
        "stages": {
            "parser": {"ok": False, "error": message},
            "compute": {
                "ok": False,
                "glyph_report_count": 0,
                "page_data_present": False,
                "assertions_enabled": False,
                "assertions_ok": False,
                "assertions": {"assertion_count": 0, "checks": []},
            },
            "layout": {
                "ok": False,
                "expected_page_count": None,
                "actual_page_count": 0,
                "dpi": 0,
                "assertions_enabled": False,
                "assertions_ok": False,
                "assertions": {"assertion_count": 0, "checks": []},
            },
            "paint": {"ok": False, "checks": []},
            "diagnostics": {"ok": False, "enabled": False, "details": {}},
        },
        "artifacts": {
            "pdf_bytes": 0,
            "pdf_sha256": None,
            "image_count": 0,
            "image_sha256": [],
            "stability_hash_expected": fixture.payload.get("stability_hash"),
            "stability_hash_actual": None,
            "stability_hash_ok": False,
        },
        "ok": False,
    }


def run(
    fixtures_dir: Path,
    out_path: Path,
    selected_ids: Optional[List[str]],
    selected_labels: Optional[List[str]],
    emit_json: bool,
    update_stability: bool,
    jobs: int,
    emit_artifacts_dir: Optional[Path],
) -> int:
    fixtures = _load_fixture_files(fixtures_dir)
    selected = _fixture_selection(fixtures, selected_ids, selected_labels)
    if not selected:
        raise ValueError(
            f"no fixtures selected (fixtures={selected_ids or []}, labels={selected_labels or []})"
        )

    fixture_results: List[Dict[str, Any]] = []
    result_by_id: Dict[str, Dict[str, Any]] = {}
    if jobs <= 1 or len(selected) <= 1:
        for fixture in selected:
            try:
                item = _run_fixture(
                    fixture,
                    update_stability=update_stability,
                    artifacts_dir=emit_artifacts_dir,
                )
            except Exception as exc:  # noqa: BLE001
                item = _fixture_exception_result(fixture, exc)
            fixture_results.append(item)
            result_by_id[item["id"]] = item
    else:
        with ThreadPoolExecutor(max_workers=jobs) as pool:
            fut_to_fixture = {
                pool.submit(
                    _run_fixture,
                    fixture,
                    update_stability,
                    emit_artifacts_dir,
                ): fixture
                for fixture in selected
            }
            for fut in as_completed(fut_to_fixture):
                fixture = fut_to_fixture[fut]
                try:
                    item = fut.result()
                except Exception as exc:  # noqa: BLE001
                    item = _fixture_exception_result(fixture, exc)
                fixture_results.append(item)
                result_by_id[item["id"]] = item

        selected_order = {fixture.fixture_id: index for index, fixture in enumerate(selected)}
        fixture_results.sort(key=lambda item: selected_order.get(str(item.get("id")), 10**9))

    if update_stability:
        _update_stability_hashes(selected, result_by_id)

    total = len(fixture_results)
    passed = sum(1 for item in fixture_results if item["ok"])
    report = {
        "schema": "fullbleed.css_fixture_suite.v1",
        "ok": passed == total,
        "summary": {
            "fixtures_total": total,
            "fixtures_passed": passed,
            "fixtures_failed": total - passed,
            "jobs": jobs,
            "selected_labels": selected_labels or [],
        },
        "fixtures": fixture_results,
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
    if emit_json:
        print(json.dumps(report, ensure_ascii=True))
    else:
        print(f"ok={report['ok']} fixtures={total} passed={passed} out={out_path}")
    return 0 if report["ok"] else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Run CSS parity fixture suite.")
    parser.add_argument(
        "--fixtures-dir",
        default="_css_working/fixtures",
        help="Directory containing fixture JSON files.",
    )
    parser.add_argument(
        "--out",
        default="_css_working/css_fixture_report.json",
        help="Output report path.",
    )
    parser.add_argument(
        "--fixtures",
        default="",
        help="Comma-separated fixture ids to run (default: all).",
    )
    parser.add_argument(
        "--labels",
        default="",
        help="Comma-separated fixture labels to run (default: all labels).",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=1,
        help="Parallel worker count for fixture execution.",
    )
    parser.add_argument(
        "--update-stability",
        action="store_true",
        help="Write current stability_hash into each selected fixture JSON.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit full report JSON to stdout.",
    )
    parser.add_argument(
        "--emit-artifacts-dir",
        default="",
        help="Optional directory to emit per-fixture output.pdf/output_page*.png/hashes.json.",
    )
    args = parser.parse_args()

    ids = [item.strip() for item in args.fixtures.split(",") if item.strip()]
    labels = [item.strip() for item in args.labels.split(",") if item.strip()]
    return run(
        fixtures_dir=Path(args.fixtures_dir),
        out_path=Path(args.out),
        selected_ids=ids or None,
        selected_labels=labels or None,
        emit_json=args.json,
        update_stability=args.update_stability,
        jobs=max(1, int(args.jobs)),
        emit_artifacts_dir=Path(args.emit_artifacts_dir)
        if str(args.emit_artifacts_dir).strip()
        else None,
    )


if __name__ == "__main__":
    raise SystemExit(main())
