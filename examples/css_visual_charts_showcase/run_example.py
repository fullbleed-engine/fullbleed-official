from __future__ import annotations

import json
import os
import random
import time
from pathlib import Path

import fullbleed


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
OUT.mkdir(parents=True, exist_ok=True)
INTER_FONT = ROOT / "vendor" / "fonts" / "Inter-Variable.ttf"

PDF_PATH = OUT / "css_visual_charts_showcase.pdf"
PERF_PATH = OUT / "css_visual_charts_showcase.perf.jsonl"
DEBUG_PATH = OUT / "css_visual_charts_showcase.debug.log"
REPORT_PATH = OUT / "css_visual_charts_showcase.run_report.json"
PNG_STEM = "css_visual_charts_showcase"

RECORD_COUNT = 72


def _env_truthy(name: str, default: bool = False) -> bool:
    raw = os.getenv(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def _env_int(name: str, default: int) -> int:
    raw = os.getenv(name, "").strip()
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        return default


def _clean_outputs() -> None:
    for path in (PDF_PATH, PERF_PATH, DEBUG_PATH, REPORT_PATH):
        path.unlink(missing_ok=True)
    for png in OUT.glob(f"{PNG_STEM}_page*.png"):
        png.unlink(missing_ok=True)


def _heat_color(v: int) -> str:
    v = max(0, min(100, v))
    r = int(40 + (v * 2.15))
    g = int(40 + (v * 1.85))
    b = int(255 - (v * 2.0))
    r = max(0, min(255, r))
    g = max(0, min(255, g))
    b = max(0, min(255, b))
    return f"rgb({r}, {g}, {b})"


def generate_data() -> dict[str, object]:
    rng = random.Random(1776)
    months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    regions = ["North", "South", "East", "West", "Central", "Mid-Atlantic", "Gulf", "Pacific"]
    channels = ["Web", "Email", "Partner", "Retail", "Field"]
    statuses = ["new", "review", "approved", "hold"]

    monthly = [int(45 + rng.random() * 65) for _ in months]
    mix_vals = [int(10 + rng.random() * 40) for _ in channels]
    mix_total = sum(mix_vals)
    mix = [
        {"name": channels[idx], "pct": round((value / mix_total) * 100.0, 1)}
        for idx, value in enumerate(mix_vals)
    ]

    gauges = [
        {"name": "Retention", "pct": 82},
        {"name": "Activation", "pct": 67},
        {"name": "Fulfillment", "pct": 74},
        {"name": "SLA", "pct": 91},
    ]

    heatmap: list[list[int]] = []
    for _ in regions:
        row = [int(8 + rng.random() * 92) for _ in months]
        heatmap.append(row)

    timeline = [
        {"title": "Scope Freeze", "day": "Mar 03"},
        {"title": "QA Burn-In", "day": "Apr 12"},
        {"title": "Pilot Launch", "day": "May 08"},
        {"title": "Regional Ramp", "day": "Jun 02"},
        {"title": "General Availability", "day": "Jul 15"},
    ]

    records: list[dict[str, object]] = []
    for i in range(1, RECORD_COUNT + 1):
        score = int(20 + rng.random() * 80)
        amount = round(250 + rng.random() * 12750, 2)
        velocity = int(5 + rng.random() * 35)
        status = statuses[(i - 1) % len(statuses)]
        records.append(
            {
                "id": i,
                "name": f"Account-{i:03d}",
                "region": regions[(i - 1) % len(regions)],
                "status": status,
                "amount": amount,
                "score": score,
                "velocity": velocity,
            }
        )

    return {
        "months": months,
        "monthly": monthly,
        "mix": mix,
        "gauges": gauges,
        "regions": regions,
        "heatmap": heatmap,
        "timeline": timeline,
        "records": records,
    }


def _bar_color(index: int) -> str:
    colors = ["#31b0ff", "#2ee67f", "#7f6bff", "#ffd84c", "#ff6f6f", "#56f0ff"]
    return colors[index % len(colors)]


def _tone_class(index: int) -> str:
    return f"tone-{index % 6}"


def build_html(data: dict[str, object], fancy: bool = False) -> str:
    months: list[str] = data["months"]  # type: ignore[assignment]
    monthly: list[int] = data["monthly"]  # type: ignore[assignment]
    mix: list[dict[str, object]] = data["mix"]  # type: ignore[assignment]
    gauges: list[dict[str, object]] = data["gauges"]  # type: ignore[assignment]
    regions: list[str] = data["regions"]  # type: ignore[assignment]
    heatmap: list[list[int]] = data["heatmap"]  # type: ignore[assignment]
    timeline: list[dict[str, str]] = data["timeline"]  # type: ignore[assignment]
    records: list[dict[str, object]] = data["records"]  # type: ignore[assignment]

    max_month = max(monthly) if monthly else 1
    throughput_rows = []
    for idx, value in enumerate(monthly):
        pct = int((value / max_month) * 100)
        tone = _tone_class(idx)
        fill_style = (
            f'--pct:{pct};'
            if fancy
            else f'transform:scaleX({pct / 100.0:.3f});'
        )
        throughput_rows.append(
            (
                '<div class="throughput-row">'
                f'<span class="tp-month">{months[idx]}</span>'
                '<div class="tp-track">'
                f'<div class="tp-fill {tone}" style="{fill_style}"></div>'
                "</div>"
                f'<strong class="tp-value">{value}</strong>'
                "</div>"
            )
        )

    mix_rows = []
    for idx, part in enumerate(mix):
        pct = float(part["pct"])
        tone = _tone_class(idx + 1)
        seg_style = (
            f'--pct:{pct:.2f};'
            if fancy
            else f'transform:scaleX({pct / 100.0:.3f});'
        )
        mix_rows.append(
            (
                '<div class="mix-row">'
                f'<span class="mix-name">{part["name"]}</span>'
                '<div class="mix-track"><div class="mix-seg '
                f'{tone}" style="{seg_style}"></div></div>'
                f'<strong>{part["pct"]:.1f}%</strong>'
                "</div>"
            )
        )

    gauges_html = []
    for idx, g in enumerate(gauges):
        tone = _bar_color(idx + 2)
        gauge_pct_style = f"--pct:{int(g['pct'])};border-color:{tone};"
        gauges_html.append(
            (
                '<article class="gauge-card">'
                f'<div class="gauge{" fancy-gauge" if fancy else ""}" style="{gauge_pct_style}">'
                f'<span class="gauge-value">{g["pct"]}%</span>'
                "</div>"
                f'<div class="gauge-strip"><span style="width:{g["pct"]}%;background:{tone};"></span></div>'
                f'<h4>{g["name"]}</h4>'
                "</article>"
            )
        )

    heat_rows = []
    for ridx, row in enumerate(heatmap):
        cells = []
        for value in row:
            cells.append(
                f'<td class="heat-cell" style="background:{_heat_color(value)};" title="{value}"><span>&nbsp;</span></td>'
            )
        heat_rows.append(
            (
                "<tr>"
                f'<th class="heat-label">{regions[ridx]}</th>'
                + "".join(cells)
                + "</tr>"
            )
        )

    timeline_html = []
    for idx, item in enumerate(timeline):
        timeline_html.append(
            (
                f'<div class="milestone{" is-last" if idx == len(timeline) - 1 else ""}">'
                f'<span class="dot"></span><h5>{item["title"]}</h5><p>{item["day"]}</p>'
                "</div>"
            )
        )

    mini_cards = []
    for rec in records:
        meter_style = (
            f'--score:{int(rec["score"])};width:calc(var(--score) * 1%);'
            if fancy
            else f'width:{rec["score"]}%;'
        )
        mini_cards.append(
            (
                f'<article class="mini-card status-{rec["status"]}">'
                '<div class="mini-top">'
                f'<h5>{rec["name"]}</h5>'
                f'<span>{rec["region"]}</span>'
                "</div>"
                '<p class="mini-stats">'
                f'${rec["amount"]:,.2f} | score {rec["score"]}% | {rec["velocity"]}d'
                "</p>"
                '<div class="mini-meter">'
                f'<span style="{meter_style}"></span>'
                "</div>"
                "</article>"
            )
        )
    kpi_total = sum(float(rec["amount"]) for rec in records)
    kpi_avg = kpi_total / len(records)
    kpi_high = max(int(rec["score"]) for rec in records)

    return (
        "<!doctype html><html><body>"
        '<main class="report">'
        '<section class="page page-intro">'
        '<header class="hero">'
        '<div class="hero-left">'
        '<h1>CSS Visual Charts Showcase</h1>'
        '<p>Deterministic chart-heavy fixture for visual parity validation in the fixed-point renderer.</p>'
        "</div>"
        '<div class="hero-right">'
        '<div class="kpi"><span>Total Pipeline</span>'
        f'<strong>${kpi_total:,.0f}</strong></div>'
        '<div class="kpi"><span>Average Account</span>'
        f'<strong>${kpi_avg:,.0f}</strong></div>'
        '<div class="kpi"><span>Peak Score</span>'
        f"<strong>{kpi_high}%</strong></div>"
        "</div>"
        "</header>"
        '<section class="grid-2">'
        '<article class="panel">'
        "<h3>Monthly Throughput</h3>"
        '<div class="throughput-rows">'
        + "".join(throughput_rows)
        + "</div>"
        "</article>"
        '<article class="panel">'
        "<h3>Channel Mix</h3>"
        '<div class="mix-bars">'
        + "".join(mix_rows)
        + "</div>"
        "</article>"
        "</section>"
        '<section class="grid-2">'
        '<article class="panel">'
        "<h3>Operational Gauges</h3>"
        '<div class="gauges">'
        + "".join(gauges_html)
        + "</div>"
        "</article>"
        '<article class="panel">'
        "<h3>Release Timeline</h3>"
        '<div class="timeline">'
        + "".join(timeline_html)
        + "</div>"
        "</article>"
        "</section>"
        "</section>"
        '<section class="page page-heat">'
        '<section class="panel">'
        "<h3>Regional Heatmap</h3>"
        '<table class="heat-table"><thead><tr><th class="heat-corner">Region</th>'
        + "".join(f"<th>{m}</th>" for m in months)
        + "</tr></thead><tbody>"
        + "".join(heat_rows)
        + "</tbody></table>"
        "</section>"
        "</section>"
        '<section class="page page-cards">'
        '<section class="panel">'
        "<h3>Record Density Grid</h3>"
        '<div class="mini-grid">'
        + "".join(mini_cards)
        + "</div>"
        "</section>"
        "</section>"
        + "</main></body></html>"
    )


def build_css(fancy: bool = False) -> str:
    base_css = """
@page { size: 8.5in 11in; margin: 0.25in; }

:root {
  --bg0: #0a1022;
  --bg1: #121a34;
  --panel: rgba(17, 27, 54, 0.86);
  --line: rgba(144, 162, 214, 0.25);
  --ink: #ecf1ff;
  --muted: #a6b0cf;
  --ok: #2ee67f;
  --warn: #ffd84c;
  --bad: #ff6f6f;
  --radius: 10pt;
  --gap: 8pt;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  font-family: Inter, Helvetica, Arial, sans-serif;
  color: var(--ink);
  background: radial-gradient(circle at 10% 0%, #1d2f6d 0%, var(--bg0) 48%, #070b19 100%);
}

.report {
  width: min(100%, 8in);
  margin: 0 auto;
}

.page {
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.02) 0%, rgba(255, 255, 255, 0.00) 100%);
  border: 1pt solid var(--line);
  border-radius: var(--radius);
  padding: 10pt;
  overflow: hidden;
}

.page:not(:last-child) {
  break-after: page;
  margin-bottom: 8pt;
}

.hero {
  display: flex;
  justify-content: space-between;
  gap: var(--gap);
  margin-bottom: 8pt;
}

.hero-left { width: 63%; }
.hero-right { width: 35%; display: grid; grid-template-columns: repeat(3, 1fr); gap: 6pt; }

h1 {
  margin: 0;
  font-size: clamp(15pt, 17pt, 21pt);
  letter-spacing: 0.2pt;
}

h3 {
  margin: 0 0 6pt 0;
  font-size: 10.5pt;
}

h4, h5 {
  margin: 0;
  font-size: 7.5pt;
}

p {
  margin: 3pt 0 0 0;
  font-size: 7.5pt;
  color: var(--muted);
}

.kpi {
  border: 1pt solid var(--line);
  border-radius: 8pt;
  padding: 5pt 6pt;
  background: rgba(30, 44, 84, 0.65);
}

.kpi span {
  display: block;
  font-size: 6.5pt;
  color: var(--muted);
  text-transform: uppercase;
}

.kpi strong {
  display: block;
  margin-top: 2pt;
  font-size: 9pt;
}

.grid-2 {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--gap);
  margin-bottom: var(--gap);
}

.panel {
  border: 1pt solid var(--line);
  border-radius: 8pt;
  padding: 8pt;
  background: var(--panel);
  overflow: hidden;
}

.tone-0 { background-color: #31b0ff; }
.tone-1 { background-color: #2ee67f; }
.tone-2 { background-color: #7f6bff; }
.tone-3 { background-color: #ffd84c; }
.tone-4 { background-color: #ff6f6f; }
.tone-5 { background-color: #56f0ff; }

.throughput-rows {
  margin-top: 2pt;
}

.throughput-row {
  display: grid;
  grid-template-columns: 24pt 1fr 22pt;
  align-items: center;
  gap: 6pt;
  padding: 2pt 0;
  border-bottom: 1pt solid rgba(255, 255, 255, 0.08);
}

.tp-month {
  font-size: 6.8pt;
  color: #dbe4ff;
}

.tp-track {
  display: block;
  width: 120pt;
  height: 8pt;
  border-radius: 6pt;
  overflow: hidden;
  border: 1pt solid rgba(255, 255, 255, 0.18);
  background: rgba(0, 0, 0, 0.18);
}

.tp-fill {
  display: block;
  width: 120pt;
  height: 100%;
  background-color: #31b0ff;
  transform-origin: left center;
}

.tp-value {
  text-align: right;
  font-size: 6.8pt;
}

.mix-bars {
  margin-top: 2pt;
}

.mix-row {
  display: grid;
  grid-template-columns: 34pt 1fr 30pt;
  align-items: center;
  gap: 6pt;
  padding: 3pt 0;
  border-bottom: 1pt solid rgba(255, 255, 255, 0.08);
}

.mix-name {
  font-size: 7pt;
  color: var(--ink);
}

.mix-track {
  width: 96pt;
  height: 8pt;
  border-radius: 6pt;
  overflow: hidden;
  display: block;
  border: 1pt solid rgba(255, 255, 255, 0.2);
  background: rgba(0, 0, 0, 0.18);
}

.mix-seg {
  display: block;
  width: 96pt;
  height: 100%;
  background-color: #31b0ff;
  transform-origin: left center;
}

.mix-row strong {
  text-align: right;
  font-size: 6.8pt;
}

.gauges {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8pt;
}

.gauge-card {
  text-align: center;
}

.gauge {
  width: 62pt;
  height: 62pt;
  margin: 0 auto 4pt auto;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(128, 142, 185, 0.16);
  border: 4pt solid rgba(133, 149, 191, 0.28);
}

.gauge-value {
  font-size: 8.4pt;
  font-weight: 700;
  color: #f2f6ff;
}

.gauge-strip {
  margin: 3pt auto 4pt auto;
  width: 62pt;
  height: 5pt;
  border-radius: 999pt;
  background: rgba(170, 184, 225, 0.25);
  overflow: hidden;
}

.gauge-strip span {
  display: block;
  height: 100%;
}

.timeline {
  display: flex;
  gap: 0;
  margin-top: 2pt;
}

.milestone {
  flex: 1;
  position: relative;
  padding-right: 8pt;
}

.milestone .dot {
  display: inline-block;
  width: 8pt;
  height: 8pt;
  border-radius: 50%;
  background: #42b7ff;
  box-shadow: 0 0 0 2pt rgba(66, 183, 255, 0.22);
}

.milestone:not(.is-last)::after {
  content: "";
  position: absolute;
  top: 3.5pt;
  left: 12pt;
  right: 2pt;
  height: 1pt;
  background: rgba(135, 162, 232, 0.45);
}

.milestone h5 {
  margin-top: 4pt;
  font-size: 7pt;
}

.milestone p {
  margin-top: 2pt;
  font-size: 6.5pt;
}

.heat-table {
  width: 100%;
  border-collapse: separate;
  border-spacing: 2pt;
  table-layout: fixed;
}

.heat-table th {
  font-size: 6pt;
  color: #d5ddff;
  text-align: center;
  font-weight: 600;
}

.heat-corner {
  text-align: left !important;
  width: 56pt;
}

.heat-label {
  text-align: left !important;
  font-size: 6.5pt !important;
  color: #d5ddff;
  width: 56pt;
}

.heat-cell {
  height: 11pt;
  padding: 0;
  border-radius: 2pt;
  border: 0.4pt solid rgba(255, 255, 255, 0.24);
}

.heat-cell span {
  color: transparent;
  font-size: 1pt;
}

.mini-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 6pt;
}

.mini-card {
  border: 1pt solid rgba(187, 201, 246, 0.28);
  border-left-width: 3pt;
  border-radius: 6pt;
  padding: 5pt;
  background: rgba(9, 16, 34, 0.68);
  min-height: 44pt;
}

.status-new { border-left-color: #31b0ff; }
.status-review { border-left-color: #2ee67f; }
.status-approved { border-left-color: #ffd84c; }
.status-hold { border-left-color: #ff6f6f; }

.mini-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 3pt;
}

.mini-top h5 {
  font-size: 6.9pt;
  margin: 0;
}

.mini-top span {
  font-size: 6pt;
  color: #afbbdf;
}

.mini-stats {
  margin: 2pt 0 4pt 0;
  font-size: 6pt;
  color: #bbc7ec;
}

.mini-meter {
  height: 6pt;
  border-radius: 999pt;
  overflow: hidden;
  background: rgba(170, 184, 225, 0.22);
}

.mini-meter span {
  display: block;
  height: 100%;
  background: linear-gradient(90deg, #43b3ff 0%, #7ff0ff 100%);
}
""".strip()

    if not fancy:
        return base_css

    fancy_css = """

/* Fancy stress lane: intentionally includes advanced CSS to probe parity edges. */
body {
  background:
    radial-gradient(circle at 12% 2%, #2b4ea3 0%, #0c1430 38%, #070b19 100%),
    conic-gradient(from 120deg at 80% 10%, rgba(91, 112, 180, 0.45), rgba(38, 53, 99, 0.10), rgba(91, 112, 180, 0.45));
}

.page {
  clip-path: inset(0 round 12pt);
}

.panel {
  background: color-mix(in srgb, var(--panel) 78%, #4c6eb4);
  box-shadow: 0 8pt 22pt rgba(3, 8, 28, 0.38);
  backdrop-filter: blur(3px) saturate(1.2);
}

.tp-fill {
  transform: none !important;
  width: calc(var(--pct) * 1%);
  background: linear-gradient(90deg, #35b6ff 0%, #8df6ff 100%);
  filter: saturate(1.25);
}

.mix-seg {
  transform: none !important;
  width: calc(var(--pct) * 1%);
  background: linear-gradient(90deg, #43b3ff 0%, #7ff0ff 100%);
}

.fancy-gauge {
  border: 4pt solid rgba(133, 149, 191, 0.28);
  color: #31b0ff;
  background:
    conic-gradient(from -90deg, currentColor calc(var(--pct) * 1%), rgba(128, 142, 185, 0.16) 0%);
}

.gauge-card:nth-child(1) .fancy-gauge { color: #7f6bff; }
.gauge-card:nth-child(2) .fancy-gauge { color: #ffd84c; }
.gauge-card:nth-child(3) .fancy-gauge { color: #ff6f6f; }
.gauge-card:nth-child(4) .fancy-gauge { color: #56f0ff; }

.fancy-gauge::before {
  content: "";
  position: absolute;
  width: 42pt;
  height: 42pt;
  border-radius: 50%;
  background: rgba(128, 142, 185, 0.16);
}

.fancy-gauge .gauge-value {
  position: relative;
  z-index: 1;
}

.timeline .dot {
  background: radial-gradient(circle at 30% 30%, #8ad7ff, #2aa9ff 64%, #1164a3 100%);
  mix-blend-mode: screen;
}

.mini-card {
  background:
    linear-gradient(160deg, rgba(21, 30, 62, 0.9) 0%, rgba(9, 16, 34, 0.75) 100%);
}
""".strip()

    return f"{base_css}\n\n{fancy_css}"


def create_engine() -> fullbleed.PdfEngine:
    debug_enabled = _env_truthy("FULLBLEED_DEBUG", default=False)
    perf_enabled = _env_truthy("FULLBLEED_PERF", default=True)

    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        debug=debug_enabled,
        debug_out=str(DEBUG_PATH) if debug_enabled else None,
        perf=perf_enabled,
        perf_out=str(PERF_PATH) if perf_enabled else None,
    )
    if INTER_FONT.exists():
        bundle = fullbleed.AssetBundle()
        bundle.add_file(str(INTER_FONT), "font")
        engine.register_bundle(bundle)
    return engine


def emit_pngs(engine: fullbleed.PdfEngine, html: str, css: str) -> list[str]:
    dpi = _env_int("FULLBLEED_IMAGE_DPI", 132)

    if hasattr(engine, "render_image_pages_to_dir"):
        paths = engine.render_image_pages_to_dir(html, css, str(OUT), dpi, PNG_STEM)
        return [str(path) for path in paths]

    if hasattr(engine, "render_image_pages"):
        image_pages = engine.render_image_pages(html, css, dpi)
        results: list[str] = []
        for idx, page in enumerate(image_pages, start=1):
            path = OUT / f"{PNG_STEM}_page{idx}.png"
            path.write_bytes(page)
            results.append(str(path))
        return results

    return []


def main() -> None:
    _clean_outputs()
    data = generate_data()
    fancy_mode = _env_truthy("FULLBLEED_FANCY", default=False)
    html = build_html(data, fancy=fancy_mode)
    css = build_css(fancy=fancy_mode)
    engine = create_engine()
    emit_png = _env_truthy("FULLBLEED_EMIT_PNG", default=True)

    t0 = time.perf_counter()
    bytes_written = engine.render_pdf_to_file(html, css, str(PDF_PATH))
    render_ms = round((time.perf_counter() - t0) * 1000.0, 2)

    png_paths: list[str] = []
    raster_ms: float | None = None
    if emit_png:
        t1 = time.perf_counter()
        png_paths = emit_pngs(engine, html, css)
        raster_ms = round((time.perf_counter() - t1) * 1000.0, 2)

    report = {
        "schema": "fullbleed.css_visual_charts_showcase.v1",
        "record_count": RECORD_COUNT,
        "bytes_written": bytes_written,
        "pdf_path": str(PDF_PATH),
        "png_paths": png_paths,
        "perf_path": str(PERF_PATH) if PERF_PATH.exists() else None,
        "debug_path": str(DEBUG_PATH) if DEBUG_PATH.exists() else None,
        "render_ms": render_ms,
        "raster_ms": raster_ms,
        "emit_png": emit_png,
        "fancy_mode": fancy_mode,
    }
    REPORT_PATH.write_text(json.dumps(report, indent=2), encoding="utf-8")

    print(f"[ok] records={RECORD_COUNT} pages={len(png_paths)}")
    print(f"[ok] pdf={PDF_PATH}")
    print(f"[ok] report={REPORT_PATH}")
    if PERF_PATH.exists():
        print(f"[ok] perf={PERF_PATH}")
    if DEBUG_PATH.exists():
        print(f"[ok] debug={DEBUG_PATH}")


if __name__ == "__main__":
    main()
