from __future__ import annotations

from typing import Any

from .fb_ui import component, el

FIELD_FLAG_COMB = 1 << 24


def _is_checked(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if value is None:
        return False
    raw = str(value).strip().lower()
    return raw in {"1", "true", "yes", "y", "on", "checked", "x"}


def _stringify(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "X" if value else ""
    return str(value)


def _field_int(field: dict[str, Any], key: str, default: int = 0) -> int:
    try:
        return int(field.get(key, default))
    except (TypeError, ValueError):
        return default


def _is_comb_field(field: dict[str, Any]) -> bool:
    if bool(field.get("comb")):
        return True
    flags = _field_int(field, "field_flags", 0)
    return bool(flags & FIELD_FLAG_COMB)


def _comb_maxlen(field: dict[str, Any]) -> int:
    maxlen = _field_int(field, "text_maxlen", 0)
    if maxlen > 0:
        return maxlen
    return 0


def _normalize_comb_text(field: dict[str, Any], value: Any) -> str:
    raw = _stringify(value).strip()
    if not raw:
        return ""

    key = str(field.get("key", "")).lower()
    name = str(field.get("pdf_field_name", "")).lower()
    if "social" in key or "social security" in name:
        cleaned = "".join(ch for ch in raw if ch.isdigit())
    else:
        cleaned = "".join(ch for ch in raw if ch.isalnum())

    maxlen = _comb_maxlen(field)
    if maxlen > 0:
        cleaned = cleaned[:maxlen]
    return cleaned.upper()


def normalize_field_text(field: dict[str, Any], value: Any) -> str:
    field_type = str(field.get("field_type", "Text"))
    if field_type == "CheckBox":
        return "X" if _is_checked(value) else ""
    if field_type == "Text" and _is_comb_field(field):
        return _normalize_comb_text(field, value)
    return _stringify(value)


def _field_style(field: dict[str, Any]) -> str:
    x = float(field.get("x_pt", 0.0))
    y = float(field.get("y_pt", 0.0))
    w = float(field.get("width_pt", 0.0))
    h = float(field.get("height_pt", 0.0))
    return f"left:{x:.3f}pt;top:{y:.3f}pt;width:{w:.3f}pt;height:{h:.3f}pt;"


def _comb_cell_style(field: dict[str, Any], slot_index: int, slot_count: int) -> str:
    x = float(field.get("x_pt", 0.0))
    y = float(field.get("y_pt", 0.0))
    w = float(field.get("width_pt", 0.0))
    h = float(field.get("height_pt", 0.0))
    cell_w = (w / slot_count) if slot_count > 0 else w
    cell_x = x + (slot_index * cell_w)
    return f"left:{cell_x:.3f}pt;top:{y:.3f}pt;width:{cell_w:.3f}pt;height:{h:.3f}pt;"


def _comb_cells(field: dict[str, Any], text: str) -> list[object]:
    maxlen = _comb_maxlen(field)
    slot_count = max(maxlen, len(text), 1)
    cells: list[object] = []
    for idx, ch in enumerate(text[:slot_count]):
        cells.append(
            el(
                "div",
                ch,
                class_name="i9-field i9-field-text i9-field-comb-cell",
                style=_comb_cell_style(field, idx, slot_count),
                title=str(field.get("pdf_field_name", "")),
                data_fb_field_key=str(field.get("key", "")),
                data_fb_field_type=str(field.get("field_type", "Text")),
                data_fb_field_comb="1",
                data_fb_comb_slot=str(idx + 1),
            )
        )
    return cells


@component
def I9Field(*, field: dict[str, Any], value: Any) -> object:
    field_type = str(field.get("field_type", "Text"))
    key = str(field.get("key", ""))
    pdf_name = str(field.get("pdf_field_name", ""))
    is_comb = field_type == "Text" and _is_comb_field(field)
    checked = field_type == "CheckBox" and _is_checked(value)
    class_name = "i9-field i9-field-checkbox" if field_type == "CheckBox" else "i9-field i9-field-text"
    if checked:
        class_name = f"{class_name} checked"

    text = normalize_field_text(field, value)
    if is_comb:
        return _comb_cells(field, text)

    return el(
        "div",
        text,
        class_name=class_name,
        style=_field_style(field),
        title=pdf_name,
        data_fb_field_key=key,
        data_fb_field_type=field_type,
        data_fb_field_comb="0",
    )


@component
def I9Page(
    *,
    page_number: int,
    fields: list[dict[str, Any]],
    values: dict[str, Any],
    record_marker: str | None = None,
) -> object:
    nodes: list[object] = [
        # Feature marker intentionally emitted from an otherwise blank node.
        el(
            "header",
            class_name="i9-page-marker",
            data_fb=f"fb.feature.i9_page_{page_number}=1",
            data_fb_role="template-marker",
        ),
    ]
    if record_marker and page_number == 1:
        nodes.append(
            el(
                "div",
                record_marker,
                class_name="i9-record-marker",
                data_fb_role="record-marker",
            )
        )
    for field in fields:
        key = str(field.get("key", ""))
        field_node = I9Field(field=field, value=values.get(key))
        if isinstance(field_node, list):
            nodes.extend(field_node)
        else:
            nodes.append(field_node)

    return el(
        "section",
        nodes,
        class_name="i9-page",
        data_fb_role="i9-page",
        data_fb_page=str(page_number),
    )


@component
def I9Overlay(*, layout: dict[str, Any], values: dict[str, Any]) -> object:
    pages = layout.get("pages") or []
    fields = layout.get("fields") or []
    fields_by_page: dict[int, list[dict[str, Any]]] = {}
    for field in fields:
        try:
            page_number = int(field.get("page", 0))
        except (TypeError, ValueError):
            continue
        if page_number <= 0:
            continue
        fields_by_page.setdefault(page_number, []).append(field)

    children: list[object] = []
    record_marker = str(values.get("__record_marker", "") or "").strip()
    for page in pages:
        try:
            page_number = int(page.get("page", 0))
        except (TypeError, ValueError):
            continue
        if page_number <= 0:
            continue
        page_fields = sorted(
            fields_by_page.get(page_number, []),
            key=lambda item: int(item.get("widget_index", 0)),
        )
        children.append(
            I9Page(
                page_number=page_number,
                fields=page_fields,
                values=values,
                record_marker=record_marker,
            )
        )

    return el(
        "div",
        children,
        class_name="i9-overlay-document",
        data_fb_role="i9-overlay-document",
    )
