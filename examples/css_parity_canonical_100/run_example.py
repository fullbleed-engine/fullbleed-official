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

PAGE_SIZE = 14
TARGET_PAGES = 100
RECORD_COUNT = PAGE_SIZE * TARGET_PAGES
PDF_PATH = OUT / "css_parity_canonical_100.pdf"
PERF_PATH = OUT / "css_parity_canonical_100.perf.jsonl"
DEBUG_PATH = OUT / "css_parity_canonical_100.debug.log"
REPORT_PATH = OUT / "css_parity_canonical_100.run_report.json"
PNG_STEM = "css_parity_canonical_100"


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


def _clean_previous_outputs() -> None:
    for path in (
        PDF_PATH,
        PERF_PATH,
        DEBUG_PATH,
        REPORT_PATH,
    ):
        path.unlink(missing_ok=True)
    for png in OUT.glob(f"{PNG_STEM}_page*.png"):
        png.unlink(missing_ok=True)


def make_records(count: int = RECORD_COUNT) -> list[dict[str, object]]:
    rng = random.Random(4242)
    regions = ["Northeast", "Southeast", "Midwest", "Southwest", "West"]
    channels = ["Web", "Email", "Print", "Partner"]
    tiers = ["Bronze", "Silver", "Gold", "Platinum"]
    statuses = ["new", "review", "approved", "hold"]

    records: list[dict[str, object]] = []
    for idx in range(1, count + 1):
        amount = round(125.0 + rng.random() * 9900.0, 2)
        score = round(0.05 + rng.random() * 0.95, 3)
        due_days = int(3 + rng.random() * 27)
        rec = {
            "id": idx,
            "name": f"Account-{idx:03d}",
            "region": regions[(idx - 1) % len(regions)],
            "channel": channels[(idx - 1) % len(channels)],
            "tier": tiers[(idx - 1) % len(tiers)],
            "status": statuses[(idx - 1) % len(statuses)],
            "amount": amount,
            "score": score,
            "due_days": due_days,
            "record_key": f"REC-{20260000 + idx}",
        }
        records.append(rec)
    return records


def chunks(items: list[dict[str, object]], size: int) -> list[list[dict[str, object]]]:
    return [items[i : i + size] for i in range(0, len(items), size)]


def page_summary(records: list[dict[str, object]]) -> dict[str, str]:
    total = sum(float(item["amount"]) for item in records)
    avg = total / len(records)
    mean_score = sum(float(item["score"]) for item in records) / len(records)
    hold_count = sum(1 for item in records if item["status"] == "hold")
    return {
        "count": str(len(records)),
        "total": f"${total:,.2f}",
        "average": f"${avg:,.2f}",
        "score": f"{mean_score * 100:.1f}%",
        "holds": str(hold_count),
    }


def build_html(records: list[dict[str, object]]) -> str:
    pages = chunks(records, PAGE_SIZE)
    html: list[str] = [
        "<!doctype html><html><body>",
        '<main class="report-root">',
    ]

    for page_index, page_records in enumerate(pages, start=1):
        summary = page_summary(page_records)
        html.append(
            (
                '<section class="page">'
                '<header class="page-head">'
                '<div class="head-left">'
                f'<h1 class="title">CSS Parity Canonical :: Page {page_index:02d}</h1>'
                '<p class="subtitle">Deterministic 100-record scenario for parser -> evaluator -> calculator coverage.</p>'
                "</div>"
                '<div class="head-right">'
                f'<p class="meta">Records {((page_index - 1) * PAGE_SIZE) + 1}-{((page_index - 1) * PAGE_SIZE) + len(page_records)}</p>'
                f'<p class="meta">Rendered {RECORD_COUNT} total records</p>'
                "</div>"
                "</header>"
            )
        )
        html.append(
            (
                '<section class="kpis">'
                f'<div class="kpi"><span class="kpi-label">Page Records</span><span class="kpi-value">{summary["count"]}</span></div>'
                f'<div class="kpi"><span class="kpi-label">Page Total</span><span class="kpi-value">{summary["total"]}</span></div>'
                f'<div class="kpi"><span class="kpi-label">Average Amount</span><span class="kpi-value">{summary["average"]}</span></div>'
                f'<div class="kpi"><span class="kpi-label">Risk Mean</span><span class="kpi-value">{summary["score"]}</span></div>'
                "</section>"
            )
        )
        html.append('<section class="records-grid">')
        for rec in page_records:
            pct = max(1, min(100, int(float(rec["score"]) * 100)))
            amount_text = f'${float(rec["amount"]):,.2f}'
            html.append(
                (
                    f'<article class="record status-{rec["status"]}">'
                    f'<span class="corner">{rec["record_key"]}</span>'
                    '<div class="record-top">'
                    f'<h2 class="record-title">{rec["name"]}</h2>'
                    f'<span class="badge">{str(rec["status"]).upper()}</span>'
                    "</div>"
                    '<div class="record-facts">'
                    f'<div><span class="label">Region</span><span class="value">{rec["region"]}</span></div>'
                    f'<div><span class="label">Channel</span><span class="value">{rec["channel"]}</span></div>'
                    f'<div><span class="label">Tier</span><span class="value">{rec["tier"]}</span></div>'
                    f'<div><span class="label">Amount</span><span class="value">{amount_text}</span></div>'
                    f'<div><span class="label">Due Days</span><span class="value">{rec["due_days"]}</span></div>'
                    f'<div><span class="label">Risk</span><span class="value">{pct}%</span></div>'
                    "</div>"
                    '<div class="spark"><span style="--score-pct: '
                    f"{pct}%"
                    ';"></span></div>'
                    "</article>"
                )
            )
        html.append("</section>")
        html.append(
            (
                '<footer class="page-foot">'
                '<table class="foot-table"><tbody><tr>'
                '<td class="cell-label">Hold Count</td>'
                f'<td class="cell-value">{summary["holds"]}</td>'
                '<td class="cell-label">Layout Mode</td>'
                '<td class="cell-value">grid + flex + abs + vars + calc/min/max/clamp</td>'
                "</tr></tbody></table>"
                "</footer>"
                "</section>"
            )
        )

    html.append("</main></body></html>")
    return "".join(html)


def build_css() -> str:
    return """
@page { size: 8.5in 11in; margin: 0.35in; }

:root {
  --bg: #e0e0e0;
  --surface: #ffffff;
  --ink: #161616;
  --muted: #666666;
  --line: #d4d4d4;
  --blue: #2f52ff;
  --green: #00e819;
  --red: #f71414;
  --yellow: #f3f300;
  --radius: 8pt;
  --space: 8pt;
  --space-lg: 12pt;
  --shadow: 0 2pt 6pt rgba(0, 0, 0, 0.12);
}

* { box-sizing: border-box; }

body {
  margin: 0;
  font-family: Inter, Helvetica, Arial, sans-serif;
  background: var(--bg);
  color: var(--ink);
}

.report-root {
  width: min(100%, 7.8in);
  margin: 0 auto;
}

.page {
  padding: var(--space-lg);
  background: linear-gradient(180deg, #f7f7f7 0%, #ececec 100%);
  border: 1pt solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow);
  overflow: hidden;
  position: relative;
}

.page:not(:last-child) {
  margin-bottom: max(8pt, var(--space));
  break-after: page;
}

.page-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space);
  margin-bottom: var(--space);
}

.head-left { width: min(70%, 5.2in); }
.head-right { width: max(26%, 1.7in); text-align: right; }

.title {
  margin: 0;
  font-size: clamp(13pt, 15pt, 18pt);
  letter-spacing: 0.2pt;
}

.subtitle {
  margin: 3pt 0 0 0;
  font-size: 8pt;
  color: var(--muted);
}

.meta {
  margin: 0;
  font-size: 8pt;
  color: #4c4c4c;
}

.kpis {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space);
  margin-bottom: var(--space);
}

.kpi {
  background: var(--surface, #fff);
  border-left: 4pt solid var(--blue);
  border-radius: 6pt;
  padding: 6pt 7pt;
  box-shadow: 0 1pt 3pt rgba(0, 0, 0, 0.1);
}

.kpi-label {
  display: block;
  font-size: 7pt;
  color: var(--muted);
  text-transform: uppercase;
}

.kpi-value {
  display: block;
  margin-top: 2pt;
  font-size: 11pt;
  font-weight: 700;
}

.records-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space);
}

.record {
  position: relative;
  min-height: 70pt;
  padding: 7pt;
  border: 1pt solid #d6d6d6;
  border-radius: 7pt;
  background: var(--surface, #fff);
  box-shadow: 0 1pt 2pt rgba(0, 0, 0, 0.09);
  transform: translateY(0pt);
}

.record:nth-child(4n + 1) { border-left: 4pt solid var(--blue); }
.record:nth-child(4n + 2) { border-left: 4pt solid var(--green); }
.record:nth-child(4n + 3) { border-left: 4pt solid var(--red); }
.record:nth-child(4n + 4) { border-left: 4pt solid var(--yellow); }

.record-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 4pt;
}

.record-title {
  margin: 0;
  font-size: 9pt;
  font-weight: 700;
}

.corner {
  position: absolute;
  top: 5pt;
  right: 7pt;
  font-size: 6.5pt;
  color: #6f6f6f;
}

.badge {
  display: inline-block;
  font-size: 6.5pt;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.3pt;
  padding: 3pt 4pt;
  border-radius: 999pt;
  color: #0d0d0d;
  background: #dedede;
}

.status-new .badge { background: #b9cbff; }
.status-review .badge { background: #b6ffbf; }
.status-approved .badge { background: #ffeeb0; }
.status-hold .badge { background: #ffc3c3; }

.record-facts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 3pt;
}

.label {
  display: block;
  font-size: 6.5pt;
  color: #666666;
  text-transform: uppercase;
}

.value {
  display: block;
  font-size: 8pt;
  font-weight: 600;
}

.spark {
  margin-top: 5pt;
  height: 7pt;
  border-radius: 999pt;
  overflow: hidden;
  background: #dfdfdf;
}

.spark > span {
  display: block;
  width: var(--score-pct);
  height: 100%;
  background: linear-gradient(90deg, #3c5fff 0%, #7db8ff 100%);
}

.page-foot {
  margin-top: var(--space);
  border-top: 1pt solid #d8d8d8;
  padding-top: 4pt;
}

.foot-table {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
}

.foot-table td {
  padding: 2pt 3pt;
  border: 1pt solid #d5d5d5;
}

.cell-label {
  width: 13%;
  font-size: 7pt;
  color: #666666;
  text-transform: uppercase;
}

.cell-value {
  width: 37%;
  font-size: 8pt;
  font-weight: 600;
}
""".strip()


def create_engine() -> fullbleed.PdfEngine:
    debug_enabled = _env_truthy("FULLBLEED_DEBUG", default=False)
    perf_enabled = _env_truthy("FULLBLEED_PERF", default=True)
    jit_mode = os.getenv("FULLBLEED_JIT_MODE", "").strip() or None

    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        debug=debug_enabled,
        debug_out=str(DEBUG_PATH) if debug_enabled else None,
        perf=perf_enabled,
        perf_out=str(PERF_PATH) if perf_enabled else None,
        jit_mode=jit_mode,
    )
    if INTER_FONT.exists():
        bundle = fullbleed.AssetBundle()
        bundle.add_file(str(INTER_FONT), "font")
        engine.register_bundle(bundle)
    return engine


def emit_pngs(engine: fullbleed.PdfEngine, html: str, css: str) -> list[str]:
    dpi = _env_int("FULLBLEED_IMAGE_DPI", 120)

    if hasattr(engine, "render_finalized_pdf_image_pages_to_dir"):
        paths = engine.render_finalized_pdf_image_pages_to_dir(str(PDF_PATH), str(OUT), dpi, PNG_STEM)
        return [str(path) for path in paths]

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
    _clean_previous_outputs()
    records = make_records(RECORD_COUNT)
    html = build_html(records)
    css = build_css()
    engine = create_engine()
    emit_png = _env_truthy("FULLBLEED_EMIT_PNG", default=True)
    emit_page_data = _env_truthy("FULLBLEED_EMIT_PAGE_DATA", default=False)

    t0 = time.perf_counter()
    page_data = None
    if emit_page_data and hasattr(engine, "render_pdf_with_page_data"):
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        PDF_PATH.write_bytes(pdf_bytes)
        bytes_written = len(pdf_bytes)
    else:
        bytes_written = engine.render_pdf_to_file(html, css, str(PDF_PATH))
    render_ms = round((time.perf_counter() - t0) * 1000.0, 2)

    png_paths: list[str] = []
    raster_ms: float | None = None
    if emit_png:
        t1 = time.perf_counter()
        png_paths = emit_pngs(engine, html, css)
        raster_ms = round((time.perf_counter() - t1) * 1000.0, 2)

    report = {
        "schema": "fullbleed.css_parity_canonical_100.v1",
        "record_count": RECORD_COUNT,
        "target_pages": TARGET_PAGES,
        "page_size": PAGE_SIZE,
        "bytes_written": bytes_written,
        "pages_rendered": len(png_paths),
        "pdf_path": str(PDF_PATH),
        "png_paths": png_paths,
        "perf_path": str(PERF_PATH) if PERF_PATH.exists() else None,
        "debug_path": str(DEBUG_PATH) if DEBUG_PATH.exists() else None,
        "render_ms": render_ms,
        "raster_ms": raster_ms,
        "emit_png": emit_png,
        "emit_page_data": emit_page_data,
        "page_data": page_data if isinstance(page_data, dict) else None,
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
