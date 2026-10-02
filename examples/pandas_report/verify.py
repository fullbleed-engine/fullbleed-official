#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify the example's real PDFs, pagination, escaping, and repeatable output."""
from __future__ import annotations

import argparse
import hashlib
from importlib import metadata, util
import json
from pathlib import Path
import re
import subprocess
import sys

import fullbleed
import pandas as pd

spec = util.spec_from_file_location("pandas_report_example", Path(__file__).with_name("report.py"))
assert spec is not None and spec.loader is not None
example = util.module_from_spec(spec)
spec.loader.exec_module(example)
ROOT = example.ROOT
dataframe_html = example.dataframe_html
font_path = example.font_path
load_data = example.load_data
render_report = example.render_report


def page_texts(path: Path) -> list[str]:
    return [p["text"] for p in fullbleed.extract_pdf_page_texts(str(path))["pages"]]


def check_rows(path: Path, skus: list[str]) -> list[str]:
    pages = page_texts(path)
    text = "\n".join(pages)
    found = re.findall(r"\b(?:GL|LONG)-\d{3}\b", text)
    assert found == skus, "A row was dropped, duplicated, or reordered."
    for number, page in enumerate(pages, 1):
        assert "SKU" in page and "Restocked" in page, "A table page is missing its repeated header."
        assert f"{number} / {len(pages)}" in page, "A page is missing its correct footer."
    return pages


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("output/pandas-report-verification"))
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    checks = []
    command = [sys.executable, "-I", str(ROOT / "report.py")]
    # Separate interpreters exercise cold startup and avoid sharing engine state.
    for folder in ["sample", "replay"]:
        result = subprocess.run([*command, "--out", str(out / folder)],
                                capture_output=True, text=True, encoding="utf-8", check=True)
        (out / (folder + ".log")).write_text(result.stdout + result.stderr, encoding="utf-8")
    pdf = out / "sample/report.pdf"
    assert pdf.read_bytes() == (out / "replay/report.pdf").read_bytes(), "PDF replay changed."
    sample = load_data(ROOT / "products.csv")
    pages = check_rows(pdf, sample["sku"].tolist())
    assert len(pages) == 2, "The published sample's page budget changed."
    assert "München desk mat / Original" in "\n".join(pages)
    assert "$150,236.25" in pages[0] and "6,040" in pages[0], "Sample totals changed."
    checks += ["40 rows preserved in order", "two-page sample with repeated headers and footers",
               "sample totals and Unicode text", "identical PDF bytes across separate processes"]

    # Pandas' nullable integer, string, float, and datetime columns use different NA sentinels.
    nullable = pd.DataFrame({
        "Integer": pd.Series([1, pd.NA], dtype="Int64"),
        "Text": pd.Series(["<b>café & crème</b>", pd.NA], dtype="string"),
        "Float": [1.5, float("nan")],
        "Date": pd.to_datetime(["2026-09-01", None]),
    })
    original = nullable.copy(deep=True)
    html = dataframe_html(nullable)
    pd.testing.assert_frame_equal(nullable, original)
    assert html.count("—") == 4 and "&lt;NA&gt;" not in html and "NaT" not in html
    assert "&lt;b&gt;café &amp; crème&lt;/b&gt;" in html
    checks.append("nullable values normalize without mutating the DataFrame")

    edge = sample.head(3).copy(deep=True)
    edge.loc[0, "product"] = "<b>café & crème</b>"
    edge.loc[1, "net_sales_cents"] = -12345
    edge.loc[2, "last_restock"] = pd.NaT
    before = edge.copy(deep=True)
    render_report(edge, out / "edge", title="Report <draft> & review")
    pd.testing.assert_frame_equal(edge, before)
    edge_text = "\n".join(check_rows(out / "edge/report.pdf", edge["sku"].tolist()))
    assert "<b>café & crème</b>" in edge_text and "-$123.45" in edge_text and "—" in edge_text
    assert "<b>café" not in (out / "edge/report.html").read_text(encoding="utf-8")
    checks.append("literal markup, accented text, negative amounts, and missing dates survive rendering")

    long = pd.concat([sample] * 3, ignore_index=True)
    long["sku"] = [f"LONG-{i + 1:03d}" for i in range(len(long))]
    long.loc[::7, "product"] = "Atlas notebook with a durable linen cover and a pocket for loose research notes"
    render_report(long, out / "long")
    long_pages = check_rows(out / "long/report.pdf", long["sku"].tolist())
    assert len(long_pages) > 2 and "research notes" in "\n".join(long_pages)
    checks.append("120 rows with wrapped names paginate without dropping or duplicating records")

    for label, bad in [("empty", sample.iloc[:0]), ("duplicate", pd.concat([sample.head(1)] * 2)),
                       ("missing", sample.drop(columns="sku"))]:
        source = out / (label + ".csv")
        bad.to_csv(source, index=False, date_format="%Y-%m-%d")
        try:
            load_data(source)
        except ValueError:
            pass
        else:
            raise AssertionError(f"Invalid {label} data was accepted.")
    checks.append("empty data, duplicate SKUs, and missing columns are rejected")

    for folder in ["sample", "edge", "long"]:
        result = subprocess.run([sys.executable, "-I", "-m", "fullbleed", "--json-only", "verify",
            "--html", str(out / folder / "report.html"), "--css", str(out / folder / "report.css"),
            "--asset", font_path(), "--fail-on", "overflow", "--fail-on", "missing-glyphs",
            "--fail-on", "font-subst"], capture_output=True, text=True, encoding="utf-8")
        (out / (folder + "-diagnostics.json")).write_text(result.stdout, encoding="utf-8")
        assert result.returncode == 0, f"{folder} diagnostics failed: {result.stdout} {result.stderr}"
    checks.append("source overflow, missing-glyph, and font-substitution gates pass")
    report = dict(ok=True, engine=metadata.version("fullbleed"), pandas=metadata.version("pandas"),
        sample_pages=len(pages), long_pages=len(long_pages), sample_rows=40, long_rows=120,
        sample_sha256=hashlib.sha256(pdf.read_bytes()).hexdigest(), checks=checks,
        scope="These fixtures, fonts, and versions; inspect previews after changing data or CSS. No standards-conformance claim.")
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
