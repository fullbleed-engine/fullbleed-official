from __future__ import annotations

from typing import Any

from .fb_ui import el
from .primitives import MetricCard, Page


def _inline_mark() -> object:
    return el(
        "svg",
        el("rect", x="10", y="10", width="108", height="108", rx="14", fill="#ffffff", stroke="#16202e", stroke_width="7"),
        el("path", d="M31 88V32h24c16 0 26 9 26 23 0 15-10 24-26 24h-9v9H31z", fill="#e23d52"),
        el("path", d="M84 32h22v56H92V45h-8V32z", fill="#16202e"),
        viewBox="0 0 128 128",
        role="img",
        aria_label="Fullbleed inline SVG mark",
        class_name="ref-cover-mark",
    )


def CoverPage(data: dict[str, Any], coverage: dict[str, Any]) -> object:
    document = data["document"]
    metrics = list(data["metrics"])
    coverage_metric = {
        "label": "CSS fixture lane",
        "value": coverage.get("summary", {}).get("fixtures_label", "available"),
        "detail": coverage.get("summary", {}).get("fixtures_detail", "Coverage snapshot emitted when local reports exist"),
    }
    return Page(
        el(
            "div",
            _inline_mark(),
            el("p", document["version"], class_name="ref-cover-version"),
            class_name="ref-cover-topline",
        ),
        el(
            "header",
            el("p", "Canonical reference", class_name="ref-cover-kicker"),
            el("h1", document["title"], class_name="ref-cover-title"),
            el("p", document["subtitle"], class_name="ref-cover-subtitle"),
            class_name="ref-cover-header",
        ),
        el(
            "section",
            [MetricCard(metric, tone="warm" if index == 0 else "neutral") for index, metric in enumerate(metrics)],
            MetricCard(coverage_metric, tone="cool"),
            class_name="ref-cover-metrics",
        ),
        el(
            "footer",
            el("p", document["audience"]),
            el("p", document["owner"], class_name="ref-cover-owner"),
            class_name="ref-cover-footer",
        ),
        class_name="ref-cover-page",
        page_id="cover",
    )
