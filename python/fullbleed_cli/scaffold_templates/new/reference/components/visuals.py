from __future__ import annotations

from typing import Any

from .fb_ui import el
from .primitives import Bar, InlineGauge, Page, PageHeader, Panel


def _heatmap(data: dict[str, Any]) -> object:
    header = el("tr", el("th", "Area"), [el("th", column, scope="col") for column in data["columns"]])
    rows = []
    for row in data["rows"]:
        cells = [el("th", row["label"], scope="row")]
        for value in row["values"]:
            cells.append(el("td", str(value), class_name=f"ref-heat ref-heat-{value}"))
        rows.append(el("tr", cells))
    return el(
        "table",
        el("caption", "Static PDF readiness heatmap"),
        el("thead", header),
        el("tbody", rows),
        class_name="ref-heatmap",
    )


def _timeline(items: list[dict[str, Any]]) -> object:
    return el(
        "ol",
        [
            el(
                "li",
                el("span", item["date"], class_name="ref-timeline-date"),
                el("strong", item["label"]),
                el("span", item["detail"]),
            )
            for item in items
        ],
        class_name="ref-timeline",
    )


def _svg_plot() -> object:
    points = [
        ("22", "88"),
        ("58", "70"),
        ("94", "48"),
        ("130", "54"),
        ("166", "34"),
        ("202", "28"),
    ]
    path = "M " + " L ".join(f"{x} {y}" for x, y in points)
    return el(
        "figure",
        el(
            "svg",
            el("rect", x="0", y="0", width="224", height="112", rx="8", fill="#f7f5ef"),
            el("path", d="M20 92H208", fill="none", stroke="#cfd8e3", stroke_width="2"),
            el("path", d="M20 16V92", fill="none", stroke="#cfd8e3", stroke_width="2"),
            el("path", d=path, fill="none", stroke="#e23d52", stroke_width="4"),
            [el("circle", cx=x, cy=y, r="4", fill="#2765b0") for x, y in points],
            viewBox="0 0 224 112",
            role="img",
            aria_label="Inline SVG trend plot",
            class_name="ref-plot-svg",
        ),
        el("figcaption", "Inline SVG plot"),
        class_name="ref-plot",
    )


def VisualsPage(data: dict[str, Any], raster_data_uri: str) -> object:
    chart_rows = data["chart"]["rows"]
    return Page(
        PageHeader(
            "Visuals",
            "Images, SVG, charts, and CSS paint",
            "This page exercises the visual surfaces that matter in static reports.",
        ),
        el(
            "section",
            Panel(
                [Bar(row["label"], row["value"]) for row in chart_rows],
                title="CSS bar chart",
                class_name="ref-panel-bars",
            ),
            Panel(
                _heatmap(data["heatmap"]),
                title="Table heatmap",
                class_name="ref-panel-heatmap",
            ),
            class_name="ref-two-column",
        ),
        el(
            "section",
            Panel(
                el("div", el("img", src="brand-mark.svg", alt="Bundled SVG brand mark", class_name="ref-asset-img"), el("span", "SVG via AssetBundle"), class_name="ref-asset-row"),
                el("div", el("img", src="reference-pattern.svg", alt="Bundled SVG reference pattern", class_name="ref-pattern-img"), el("span", "Second registered SVG"), class_name="ref-asset-row"),
                el("div", el("img", src=raster_data_uri, alt="Embedded raster swatch", class_name="ref-raster-img"), el("span", "PNG data URI"), class_name="ref-asset-row"),
                title="Image paths",
                class_name="ref-panel-assets",
            ),
            Panel(
                InlineGauge(88, "Render readiness"),
                _svg_plot(),
                title="Inline SVG",
                class_name="ref-panel-inline-svg",
            ),
            class_name="ref-two-column ref-visual-bottom",
        ),
        class_name="ref-visuals-page",
        page_id="visuals",
    )


def TimelinePage(data: dict[str, Any]) -> object:
    return Page(
        PageHeader(
            "Visuals",
            "Release timeline",
            "A compact paged timeline keeps long visual narratives intentional instead of accidental.",
        ),
        Panel(
            _timeline(data["timeline"]),
            title="Release timeline",
            class_name="ref-panel-timeline",
        ),
        class_name="ref-timeline-page",
        page_id="visual-timeline",
    )
