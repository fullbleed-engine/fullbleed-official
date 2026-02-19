# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Finalize command handlers (template composition workflows)."""

import json
import sys
from pathlib import Path


def _emit_error(*, args, code: str, message: str, **extra) -> None:
    if getattr(args, "json", False):
        payload = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": code,
            "message": message,
        }
        payload.update(extra)
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] {code}: {message}\n")
    raise SystemExit(2)


def _parse_page_map(value):
    if not value:
        return None
    candidate = Path(value)
    if candidate.exists():
        raw = candidate.read_text(encoding="utf-8")
    else:
        raw = value
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"invalid --page-map JSON: {exc}") from exc
    if not isinstance(data, list):
        raise ValueError("--page-map must be a JSON list of [template_page, overlay_page] pairs")
    pairs = []
    for i, item in enumerate(data):
        if not isinstance(item, list) or len(item) != 2:
            raise ValueError(f"--page-map item {i} must be [template_page, overlay_page]")
        try:
            tpl_i = int(item[0])
            ovl_i = int(item[1])
        except (TypeError, ValueError) as exc:
            raise ValueError(f"--page-map item {i} indices must be integers") from exc
        pairs.append((tpl_i, ovl_i))
    return pairs


def _read_json_or_path(value):
    if value is None:
        raise ValueError("value is required")
    candidate = Path(value)
    if candidate.exists():
        return json.loads(candidate.read_text(encoding="utf-8-sig"))
    return json.loads(value)


def _load_template_catalog(value):
    path = Path(value)
    if path.is_dir():
        entries = []
        for pdf_path in sorted(path.glob("*.pdf")):
            entries.append((pdf_path.stem, str(pdf_path)))
        if not entries:
            raise ValueError(f"no .pdf templates found in directory: {path}")
        return entries

    data = _read_json_or_path(value)
    if isinstance(data, dict):
        if "templates" in data:
            data = data["templates"]
        elif "by_id" in data and isinstance(data["by_id"], dict):
            data = [
                {"template_id": key, "pdf_path": val}
                for key, val in sorted(data["by_id"].items(), key=lambda kv: kv[0])
            ]
    if not isinstance(data, list):
        raise ValueError("--templates must be a directory or JSON list/object")

    out = []
    for i, item in enumerate(data):
        if not isinstance(item, dict):
            raise ValueError(f"template catalog item {i} must be an object")
        template_id = item.get("template_id") or item.get("id")
        pdf_path = item.get("pdf_path") or item.get("path") or item.get("pdf")
        if not template_id or not isinstance(template_id, str):
            raise ValueError(f"template catalog item {i} missing string template_id")
        if not pdf_path or not isinstance(pdf_path, str):
            raise ValueError(f"template catalog item {i} missing string pdf_path")
        out.append((template_id, pdf_path))
    if not out:
        raise ValueError("template catalog is empty")
    return out


def _load_compose_plan(value):
    data = _read_json_or_path(value)
    if isinstance(data, dict):
        data = data.get("pages", data.get("plan", data))
    if not isinstance(data, list):
        raise ValueError("--plan must be a JSON list or object with 'pages'")

    out = []
    for i, item in enumerate(data):
        if not isinstance(item, dict):
            raise ValueError(f"plan item {i} must be an object")
        template_id = item.get("template_id")
        template_page = item.get("template_page")
        overlay_page = item.get("overlay_page")
        dx = item.get("dx", 0.0)
        dy = item.get("dy", 0.0)

        if template_id is None and isinstance(item.get("layers"), list):
            for layer in item["layers"]:
                if not isinstance(layer, dict):
                    continue
                kind = str(layer.get("kind", "")).strip().lower()
                if kind == "template":
                    template_id = layer.get("template_id", template_id)
                    template_page = layer.get("template_page", template_page)
                elif kind == "overlay":
                    overlay_page = layer.get("overlay_page", overlay_page)
                    dx = layer.get("dx", dx)
                    dy = layer.get("dy", dy)

        if not isinstance(template_id, str) or not template_id.strip():
            raise ValueError(f"plan item {i} missing string template_id")
        try:
            template_page = int(template_page)
            overlay_page = int(overlay_page)
            dx = float(dx)
            dy = float(dy)
        except (TypeError, ValueError) as exc:
            raise ValueError(
                f"plan item {i} requires numeric template_page, overlay_page, dx, dy"
            ) from exc

        out.append((template_id, template_page, overlay_page, dx, dy))

    if not out:
        raise ValueError("compose plan cannot be empty")
    return out


def cmd_finalize_stamp(args) -> None:
    """CLI handler for `fullbleed finalize stamp`."""
    template_path = Path(args.template)
    overlay_path = Path(args.overlay)
    out_path = Path(args.out)

    if not template_path.exists():
        _emit_error(args=args, code="TEMPLATE_NOT_FOUND", message=f"Template PDF not found: {template_path}")
    if not overlay_path.exists():
        _emit_error(args=args, code="OVERLAY_NOT_FOUND", message=f"Overlay PDF not found: {overlay_path}")

    try:
        page_map = _parse_page_map(args.page_map)
    except ValueError as exc:
        _emit_error(args=args, code="PAGE_MAP_INVALID", message=str(exc))

    try:
        import fullbleed

        dx = float(getattr(args, "dx", 0.0) or 0.0)
        dy = float(getattr(args, "dy", 0.0) or 0.0)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        result = fullbleed.finalize_stamp_pdf(
            str(template_path),
            str(overlay_path),
            str(out_path),
            page_map=page_map,
            dx=dx,
            dy=dy,
        )
    except Exception as exc:
        msg = str(exc)
        low = msg.lower()
        if "template pdf is encrypted" in low or ("template" in low and "encrypted" in low):
            _emit_error(args=args, code="TEMPLATE_ENCRYPTED", message=msg)
        if "overlay pdf is encrypted" in low or ("overlay" in low and "encrypted" in low):
            _emit_error(args=args, code="OVERLAY_ENCRYPTED", message=msg)
        if "page_map" in low or "page count mismatch" in low or "index out of range" in low:
            _emit_error(args=args, code="PAGE_MAP_INVALID", message=msg)
        if "pdf compose error" in low or "io error" in low:
            _emit_error(args=args, code="PDF_READ_ERROR", message=msg)
        _emit_error(args=args, code="FINALIZE_STAMP_FAILED", message=msg)

    bytes_written = out_path.stat().st_size if out_path.exists() else 0
    pages_written = int(result.get("pages_written", len(page_map or [])))
    payload = {
        "schema": "fullbleed.finalize_stamp_result.v1",
        "ok": True,
        "mode": "stamp",
        "bytes_written": int(bytes_written),
        "pages_written": pages_written,
        "outputs": {"pdf": str(out_path)},
    }
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] wrote {out_path} ({bytes_written} bytes, pages={pages_written})\n")


def cmd_finalize_compose(args) -> None:
    """CLI handler for `fullbleed finalize compose`."""
    overlay_path = Path(args.overlay)
    out_path = Path(args.out)

    if not overlay_path.exists():
        _emit_error(args=args, code="OVERLAY_NOT_FOUND", message=f"Overlay PDF not found: {overlay_path}")

    try:
        templates = _load_template_catalog(args.templates)
    except ValueError as exc:
        _emit_error(args=args, code="TEMPLATE_CATALOG_INVALID", message=str(exc))

    try:
        plan = _load_compose_plan(args.plan)
    except ValueError as exc:
        _emit_error(args=args, code="PLAN_INVALID", message=str(exc))

    try:
        import fullbleed

        out_path.parent.mkdir(parents=True, exist_ok=True)
        annotation_mode = str(getattr(args, "compose_annotation_mode", "link_only") or "link_only")
        result = fullbleed.finalize_compose_pdf(
            templates,
            plan,
            str(overlay_path),
            str(out_path),
            annotation_mode=annotation_mode,
        )
    except Exception as exc:
        msg = str(exc)
        low = msg.lower()
        if "encrypted" in low and "overlay" in low:
            _emit_error(args=args, code="OVERLAY_ENCRYPTED", message=msg)
        if "encrypted" in low and "template" in low:
            _emit_error(args=args, code="TEMPLATE_ENCRYPTED", message=msg)
        if "unknown template_id" in low:
            _emit_error(args=args, code="TEMPLATE_NOT_FOUND", message=msg)
        if "plan item" in low or "out of range" in low or "compose plan cannot be empty" in low:
            _emit_error(args=args, code="PLAN_INVALID", message=msg)
        if "template catalog cannot be empty" in low:
            _emit_error(args=args, code="TEMPLATE_CATALOG_INVALID", message=msg)
        if "pdf compose error" in low or "io error" in low:
            _emit_error(args=args, code="PDF_READ_ERROR", message=msg)
        _emit_error(args=args, code="FINALIZE_COMPOSE_FAILED", message=msg)

    bytes_written = out_path.stat().st_size if out_path.exists() else 0
    pages_written = int(result.get("pages_written", len(plan)))
    payload = {
        "schema": "fullbleed.finalize_compose_result.v1",
        "ok": True,
        "mode": "compose",
        "bytes_written": int(bytes_written),
        "pages_written": pages_written,
        "outputs": {"pdf": str(out_path)},
        "metrics": {
            "templates": len(templates),
            "plan_pages": len(plan),
            "annotation_mode": annotation_mode,
            "experimental_xobject_reuse": bool(
                getattr(args, "experimental_xobject_reuse", False)
            ),
        },
    }
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(
            f"[ok] wrote {out_path} ({bytes_written} bytes, pages={pages_written}, templates={len(templates)})\n"
        )
