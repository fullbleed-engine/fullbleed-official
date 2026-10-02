<!-- SPDX-License-Identifier: MIT -->
# A styled PDF report from pandas

Turn a product DataFrame into a two-page landscape report with embedded Inter,
summary metrics, native SVG category bars, striped rows, repeated table headers,
and page numbers. The 40-row CSV is fictional. Text remains searchable in the PDF.

## Run

Use Python 3.11 or newer in a virtual environment. From the repository root:

```bash
python -m pip install -r examples/pandas_report/requirements.txt
python examples/pandas_report/report.py --out output/pandas-report
```

Open `output/pandas-report/report.pdf`. The directory also contains the source
HTML/CSS, a PNG preview for each page, and `render.json` with the versions, page
count, PDF hash, and font hash. All rendering uses the installed Fullbleed wheel;
no browser, system PDF stack, or system fonts are required.

Pandas is an optional dependency of this example. It is not added to the Fullbleed
package. The requirements pin Fullbleed 2.5.2 and pandas 3.0.6.

## Use your data

```bash
python examples/pandas_report/report.py --csv products.csv --period "OCTOBER 2026" --out output/october
```

The CSV columns are:

| Column | Meaning |
| --- | --- |
| `sku` | Unique, nonempty product identifier |
| `product` | Product name |
| `category` | Category label |
| `units` | Nonnegative integer count |
| `net_sales_cents` | Signed integer cents in USD; `1250` renders as `$12.50` |
| `last_restock` | Date in `YYYY-MM-DD` form, or blank |

Only `last_restock` may be missing. Extra columns are ignored. An empty file,
missing required columns, duplicate SKUs, invalid dates, and missing required
values are rejected. Adapt `load_data()` and `document_html()` for another schema.
Change the fictional brand, source note, and `FOOTER` before using real data.

`dataframe_html()` builds a presentation copy and normalizes pandas' nullable
values before calling `DataFrame.to_html()`. It uses `escape=True`, `index=False`,
and `na_rep="—"`; literal `<`, `>`, and `&` stay text. Titles and category labels
are escaped separately. Format money and dates before creating the table, while
keeping numeric columns available for calculations. The source DataFrame is not
modified by rendering.

`report.css` controls page geometry, typography, colors, and column widths. The
product column wraps; numeric columns align right. The category bars compare
positive net sales, with zero fill for a negative total. Adjust this chart if your
data needs a signed axis. Review all previews after changing data or styling,
especially for wide tables, long category names, or rows taller than a page.

## Verify

```bash
python -I examples/pandas_report/verify.py --out output/pandas-report-verification
```

This exercises real PDFs: the two-page sample, a small edge-case report, and a
120-row report with wrapped names. It checks row order, repeated headers, page
numbers, totals, Unicode, literal markup, negative amounts, missing values, and
identical PDF bytes across two separate processes. It also runs the source
overflow, missing-glyph, and font-substitution diagnostic gates. Outputs and
`verification.json` are retained in the chosen directory.

The example workflow runs these checks on Windows and Linux with pandas 2.3.3
and 3.0.6, Python 3.11, and Fullbleed 2.5.2. These checks cover the supplied fixtures
and assets; they do not establish PDF standards conformance or guarantee every
possible DataFrame will fit this layout.

Fullbleed and these sources are MIT licensed. The wheel contains the Inter font
and its accompanying license.
