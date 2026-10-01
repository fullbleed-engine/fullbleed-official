# Designed document showcase

Four fictional document families with complete Python, HTML/CSS, data, and font
sources. [Open the published PDFs and previews](https://docs.fullbleed.dev/examples/).

| Example | Output | Design and rendering features |
| --- | --- | --- |
| Northstar Studio invoice | 1 page | Three embedded typefaces, grid, ruled line items, decimal totals, a high-contrast total panel |
| Common Ground review | 3 pages | Editorial cover, native SVG landscape, gradient chart bars, data-driven allocation shares, explicit page composition |
| Riverton service notice | 1 page | Cobalt palette, prominent schedule, semantic headings and lists, `pdfua1` profile |
| Hillside member statements | 3 records / 3 pages | Serif typography, a lilac balance panel, contribution charts, compiled reflow bindings |

## Run

From the repository root, with Python 3.10 or newer:

```bash
python -m pip install fullbleed==2.4.0
python examples/design_showcase/render.py --out output/design-showcase
```

The published examples were generated with the public 2.4.0 wheel. The script also
runs against the installed wheel in CI. No additional Python packages are needed.
Use `--only invoice`, `--only report`, `--only notice`, or `--only statements` to
render one family. PNG previews default to 120 DPI; change this with `--dpi`.

Each output directory contains a PDF, expanded HTML/CSS, PNG previews, and JSON
inspection results. The statements directory also contains its binding data and
compiled-template statistics. `verification.json` records the engine version,
font hashes, PDF hashes, page counts, and the checks performed.

## Make it yours

Edit `data.json` for invoice items, report metrics and allocations, and member
records. Edit each HTML/CSS pair for the page composition. Shared page dimensions,
typographic helpers, grids, and table rules live in `common.css`. Invoice totals
use Python `Decimal`; report allocation shares and chart heights come from data.

These are deliberately composed A4 pages with fixed page budgets. The report has
three explicit pages; it is not an arbitrary-length report template. Review every
page after adding content, changing fonts, or increasing the number of rows.

The statements use `render_pdf_reflow_bindings_to_file`, with one record per page
for this dataset. They use solid panel and chart fills for the compiled reflow
path. The report demonstrates gradients through the ordinary HTML/CSS path.

## Fonts and artwork

Inter is explicitly loaded from the installed Fullbleed wheel. Bebas Neue and DM
Serif Display are included in `fonts/`, together with their SIL Open Font License
texts. `fonts/sources.json` records the pinned upstream URLs and SHA-256 hashes.
The landscape is an editable vector drawing in `landscape.svg`.

The published previews are rendered from the finalized PDFs. Names, addresses,
organizations, financial figures, and service information are fictional.

## Verification scope

The script checks expected page counts, required text and record order, internal
PDF inspection, and matching PDF hashes on replay. The ordinary examples and a
first-record statement proof also pass the installed CLI's overflow,
missing-glyph, and font-substitution gates. The final compiled statement pages
are checked for content and replay consistency and must be visually reviewed.
CLI overflow checks do not establish that two elements never overlap.

The notice uses `pdfua1` and includes tags, language, and profile metadata. These
checks do **not** establish complete accessibility or independent standards
conformance. Validate the exact delivered artifact with the applicable automated
and human checks before making such a claim.
