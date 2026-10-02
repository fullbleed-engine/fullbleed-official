#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Export a pandas DataFrame as a paginated, styled Fullbleed report."""
from __future__ import annotations

import argparse
from decimal import Decimal
import hashlib
from html import escape
from importlib import metadata, resources
import json
from pathlib import Path

import fullbleed
import pandas as pd

ROOT = Path(__file__).resolve().parent
COLUMNS = ["sku", "product", "category", "units", "net_sales_cents", "last_restock"]
FOOTER = "GRIDLINE  /  OPERATIONS PULSE     |     FICTIONAL SAMPLE DATA     |     {page} / {pages}"


def font_path() -> str:
    return str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))


def load_data(path: Path) -> pd.DataFrame:
    frame = pd.read_csv(path, dtype={
        "sku": "string", "product": "string", "category": "string",
        "units": "Int64", "net_sales_cents": "Int64", "last_restock": "string",
    })
    missing = sorted(set(COLUMNS) - set(frame.columns))
    if missing:
        raise ValueError(f"Missing CSV columns: {', '.join(missing)}")
    if frame.empty:
        raise ValueError("The report needs at least one product row.")
    if frame[COLUMNS[:-1]].isna().any().any():
        raise ValueError("Only last_restock may be missing in this report's data contract.")
    if frame["sku"].duplicated().any():
        raise ValueError("Product SKUs must be unique.")
    if (frame["units"] < 0).any():
        raise ValueError("Units must be nonnegative.")
    frame["last_restock"] = pd.to_datetime(frame["last_restock"], format="%Y-%m-%d", errors="raise")
    return frame.loc[:, COLUMNS]


def dollars(cents: int) -> str:
    value = Decimal(int(cents)) / Decimal(100)
    return f"-${abs(value):,.2f}" if value < 0 else f"${value:,.2f}"


def dataframe_html(frame: pd.DataFrame) -> str:
    """Keep text escaped and normalize pandas' nullable missing-value variants."""
    display = frame.astype(object).where(frame.notna(), float("nan"))
    return display.to_html(index=False, border=0, classes="report-table", justify="left",
                           escape=True, na_rep="—", max_rows=None, max_cols=None)


def category_bars(frame: pd.DataFrame) -> str:
    totals = frame.groupby("category", sort=True)["net_sales_cents"].sum()
    maximum = max(int(totals.max()), 1)
    items = []
    for category, cents in totals.items():
        # The chart is a positive-net-sales comparison; a negative total has no filled bar.
        width = max(0, min(100, int(cents) * 100 / maximum))
        items.append(f'<div class="category"><span>{escape(str(category))}</span>'
                     f'<svg width="148" height="8" viewBox="0 0 148 8">'
                     f'<rect x="0" y="0" width="148" height="8" fill="#dce3de"/>'
                     f'<rect x="0" y="0" width="{width * 1.48:.2f}" height="8" fill="#c94e33"/>'
                     '</svg></div>')
    return "".join(items)


def document_html(frame: pd.DataFrame, *, title: str, period: str) -> str:
    display = pd.DataFrame({
        "SKU": frame["sku"], "Product": frame["product"], "Category": frame["category"],
        "Units": frame["units"].map(lambda value: f"{int(value):,}"),
        "Net sales": frame["net_sales_cents"].map(dollars),
        "Restocked": frame["last_restock"].dt.strftime("%Y-%m-%d"),
    })
    total = sum(int(value) for value in frame["net_sales_cents"])
    units = sum(int(value) for value in frame["units"])
    return f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>{escape(title)}</title></head>
<body>
<div class="masthead"><span class="brand">GRIDLINE</span><span class="period">{escape(period)} / FIELD REPORT 01</span></div>
<div class="hero"><div><p class="eyebrow">THE MONTH IN PRODUCTS</p><h1>{escape(title)}</h1>
<p class="dek">A clear view of sales, volume, and the products behind them.</p></div>
<div class="categories"><p class="eyebrow">NET SALES BY CATEGORY</p>{category_bars(frame)}</div></div>
<div class="metrics"><div class="metric primary"><p>NET SALES</p><strong>{dollars(total)}</strong></div>
<div class="metric"><p>UNITS SOLD</p><strong>{units:,}</strong></div>
<div class="metric"><p>PRODUCTS</p><strong>{len(frame):02d}</strong></div>
<div class="metric"><p>CATEGORIES</p><strong>{frame["category"].nunique():02d}</strong></div></div>
<div class="table-heading"><h2>Product ledger</h2><p>USD · — means no restock date supplied</p></div>
{dataframe_html(display)}
<p class="source-note">Source: fictional Gridline product data. Net sales are supplied in integer cents; values and categories are illustrative.</p>
</body></html>'''


def render_report(frame: pd.DataFrame, output: Path, *, title: str = "Operations pulse.",
                  period: str = "SEPTEMBER 2026") -> dict:
    output.mkdir(parents=True, exist_ok=True)
    html = document_html(frame, title=title, period=period)
    css = (ROOT / "report.css").read_text(encoding="utf-8")
    (output / "report.html").write_text(html, encoding="utf-8")
    (output / "report.css").write_text(css, encoding="utf-8")
    engine = fullbleed.PdfEngine(font_files=[font_path()], document_title=title, document_lang="en-US",
        footer_each=FOOTER, footer_x="0.55in", footer_y_from_bottom="0.25in",
        footer_font_name="Inter", footer_font_size=7, footer_color="#58645d")
    pdf = output / "report.pdf"
    engine.render_pdf_to_file(html, css, str(pdf))
    previews = list(engine.render_finalized_pdf_image_pages_to_dir(str(pdf), str(output / "preview"), 144, "report"))
    inspection = dict(fullbleed.inspect_pdf(str(pdf)))
    report = dict(engine=metadata.version("fullbleed"), pandas=metadata.version("pandas"),
                  rows=len(frame), page_count=inspection["page_count"], pdf=str(pdf), previews=previews,
                  sha256=hashlib.sha256(pdf.read_bytes()).hexdigest(),
                  font_sha256=hashlib.sha256(Path(font_path()).read_bytes()).hexdigest())
    (output / "render.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csv", type=Path, default=ROOT / "products.csv")
    parser.add_argument("--out", type=Path, default=Path("output/pandas-report"))
    parser.add_argument("--title", default="Operations pulse.")
    parser.add_argument("--period", default="SEPTEMBER 2026")
    args = parser.parse_args()
    result = render_report(load_data(args.csv), args.out, title=args.title, period=args.period)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
