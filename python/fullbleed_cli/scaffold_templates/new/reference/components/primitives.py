from __future__ import annotations

from typing import Any, Iterable

from .fb_ui import el


def class_names(*parts: Any) -> str:
    return " ".join(str(part).strip() for part in parts if str(part or "").strip())


def flatten_nodes(items: Iterable[Any]) -> list[Any]:
    out: list[Any] = []
    for item in items:
        if item is None:
            continue
        if isinstance(item, (list, tuple)):
            out.extend(flatten_nodes(item))
        else:
            out.append(item)
    return out


def Page(*children: Any, class_name: str | None = None, page_id: str | None = None) -> object:
    return el(
        "section",
        flatten_nodes(children),
        class_name=class_names("ref-page", class_name),
        data_ref_page=page_id,
    )


def PageHeader(kicker: str, title: str, subtitle: str | None = None) -> object:
    return el(
        "header",
        el("p", kicker, class_name="ref-kicker"),
        el("h2", title, class_name="ref-page-title"),
        el("p", subtitle, class_name="ref-page-subtitle") if subtitle else None,
        class_name="ref-page-header",
    )


def Panel(*children: Any, title: str | None = None, class_name: str | None = None) -> object:
    return el(
        "article",
        el("h3", title, class_name="ref-panel-title") if title else None,
        flatten_nodes(children),
        class_name=class_names("ref-panel", class_name),
    )


def Badge(text: str, tone: str = "neutral") -> object:
    return el("span", text, class_name=class_names("ref-badge", f"ref-badge-{tone}"))


def MetricCard(metric: dict[str, Any], tone: str = "neutral") -> object:
    return el(
        "article",
        el("p", metric.get("label", ""), class_name="ref-metric-label"),
        el("p", metric.get("value", ""), class_name="ref-metric-value"),
        el("p", metric.get("detail", ""), class_name="ref-metric-detail"),
        class_name=class_names("ref-metric-card", f"ref-metric-{tone}"),
    )


def KeyValueList(items: Iterable[tuple[str, Any]]) -> object:
    rows = [
        el(
            "div",
            el("dt", label, class_name="ref-kv-label"),
            el("dd", value, class_name="ref-kv-value"),
            class_name="ref-kv-row",
        )
        for label, value in items
    ]
    return el("dl", rows, class_name="ref-kv-list")


def DataTable(
    columns: list[tuple[str, str]],
    rows: Iterable[dict[str, Any]],
    *,
    caption: str | None = None,
    class_name: str | None = None,
) -> object:
    head = el(
        "thead",
        el(
            "tr",
            [el("th", label, scope="col") for key, label in columns],
        ),
    )
    body_rows = []
    for row in rows:
        row_attrs: dict[str, Any] = {}
        data_fb = row.get("_data_fb")
        if data_fb:
            row_attrs["data_fb"] = data_fb
        cells = []
        for index, (key, _label) in enumerate(columns):
            tag = "th" if index == 0 else "td"
            props = {"scope": "row"} if tag == "th" else {}
            cells.append(el(tag, row.get(key, ""), **props))
        body_rows.append(el("tr", cells, **row_attrs))
    return el(
        "table",
        el("caption", caption) if caption else None,
        head,
        el("tbody", body_rows),
        class_name=class_names("ref-table", class_name),
    )


def StepList(items: Iterable[dict[str, Any]], *, class_name: str | None = None) -> object:
    steps = [
        el(
            "li",
            el("strong", item.get("step", item.get("label", ""))),
            el("span", item.get("description", item.get("detail", ""))),
        )
        for item in items
    ]
    return el("ol", steps, class_name=class_names("ref-step-list", class_name))


def Bar(label: str, value: int | float, *, max_value: int | float = 100) -> object:
    pct = 0 if max_value == 0 else max(0, min(100, int((float(value) / float(max_value)) * 100)))
    fill_width = 3.35 * pct / 100
    return el(
        "div",
        el(
            "div",
            el("span", label, class_name="ref-bar-label"),
            el("span", f"{pct}%", class_name="ref-bar-value"),
            class_name="ref-bar-top",
        ),
        el(
            "span",
            el(
                "span",
                class_name="ref-bar-fill",
                style=f"width: {fill_width:.2f}in; height: 0.16in; display: block; background-color: #1f9f8a;",
            ),
            class_name="ref-bar-track",
        ),
        class_name="ref-bar",
    )


def InlineGauge(value: int, label: str) -> object:
    clamped = max(0, min(100, int(value)))
    dash = round(188.4 * clamped / 100, 2)
    return el(
        "figure",
        el(
            "svg",
            el("circle", cx="42", cy="42", r="30", fill="none", stroke="#e4e9ef", stroke_width="9"),
            el(
                "circle",
                cx="42",
                cy="42",
                r="30",
                fill="none",
                stroke="#1f9f8a",
                stroke_width="9",
                stroke_linecap="round",
                stroke_dasharray=f"{dash} 188.4",
            ),
            el(
                "text",
                str(clamped),
                x="42",
                y="47",
                text_anchor="middle",
                fill="#16202e",
                font_size="16",
                font_weight="800",
            ),
            viewBox="0 0 84 84",
            class_name="ref-gauge-svg",
            role="img",
            aria_label=f"{label}: {clamped} percent",
        ),
        el("figcaption", label),
        class_name="ref-gauge",
    )
