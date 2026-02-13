# Bank Statement Example

This example implements a multi-page bank statement using Fullbleed's component API, CSV data input, and per-page engine templating.

It is a reference for:
- Paginated transactional layouts.
- Per-page header/footer templates.
- Page-level aggregate calculations (`paginated_context`).
- Component-first authoring with validation outputs.

## What This Example Demonstrates

- CSV-driven statement generation (`data/statement.csv`).
- Component composition through:
  - `components/header.py`
  - `components/body.py`
- Engine-native per-page templating:
  - different margins for page 1 vs continuation pages
  - continuation header text (`header_each`)
- page-level and total-level footer metrics (`footer_each`, `footer_last`)
- Validation and diagnostics pipeline equivalent to scaffolded production flow.

## Image-to-PDF Workflow (First-Class)

This example is explicitly designed for image-led reconstruction and refinement.

Reference image:
- `source.png`

Recommended loop:
- Start from `source.png` as the visual contract.
- Build major blocks as components first (header/account details, summary, transaction table).
- Use per-page templates early so continuation pages do not drift from the intended design.
- Run `python report.py` after each change.
- Compare `output/bank_statement_page1.png` and `output/bank_statement_page2.png` against the target style.
- Increase preview precision when polishing:
  - `FULLBLEED_IMAGE_DPI=180` or higher.
- Keep JSON diagnostics green while matching visuals:
  - `output/bank_statement_component_mount_validation.json`
  - `output/bank_statement_css_layers.json`

For AI and human workflows alike, the fastest path is: image target -> component decomposition -> iterative render -> visual compare -> lock layout.

## Directory Layout

```text
examples/bank_statement/
|- report.py
|- source.png
|- data/statement.csv
|- SCAFFOLDING.md
|- components/
|  |- fb_ui.py
|  |- primitives.py
|  |- header.py
|  |- body.py
|  |- footer.py
|  `- styles/
|     |- primitives.css
|     |- header.css
|     |- body.css
|     `- footer.css
|- styles/
|  |- tokens.css
|  `- report.css
|- vendor/
|  |- css/bootstrap.min.css
|  |- fonts/Inter-Variable.ttf
|  `- icons/bootstrap-icons.svg
`- output/
```

`components/footer.py` exists for extension, while the active render composition currently mounts `Header(...)` and `Body(...)`.

## Data Contract (`data/statement.csv`)

CSV header:

```csv
row_type,key,value,date,description,amount
```

Supported `row_type` values:
- `meta`: account and statement metadata.
- `transaction`: dated ledger entries.

Required `meta` keys enforced by `load_statement_data(...)`:
- `bank_name`
- `bank_tagline`
- `contact_phone`
- `contact_email`
- `contact_website`
- `account_holder`
- `account_address_line1`
- `account_address_line2`
- `account_number`
- `routing_number`
- `statement_start`
- `statement_end`
- `beginning_balance`

Transaction requirements:
- `date` in `YYYY-MM-DD`
- `description`
- `amount` (signed decimal; positive for inflow, negative for outflow)

## Run

From repository root:

```powershell
cd examples/bank_statement
python report.py
```

## Render Outputs

Generated artifacts:
- `output/bank_statement.pdf`
- `output/bank_statement_page1.png`
- `output/bank_statement_page2.png` (when multiple pages are present)
- `output/bank_statement_component_mount_validation.json`
- `output/bank_statement_css_layers.json`

Optional artifacts (env-controlled):
- `output/bank_statement_page_data.json` (`FULLBLEED_EMIT_PAGE_DATA=1`)
- `output/bank_statement.jit.jsonl` (`FULLBLEED_DEBUG=1`)
- `output/bank_statement.perf.jsonl` (`FULLBLEED_PERF=1`)

## Engine Usage (Specific to This Example)

`report.py` configures the engine with document-level and per-page behavior:

- Base page geometry:
  - `page_width="8.5in"`
  - `page_height="11in"`
  - `margin="0in"`
- `page_margins`:
  - page `1`: full-bleed style first page.
  - page `2` and `"n"`: continuation margins with room for running header/footer text.
- Running continuation header:
  - `header_each="Bank Statement Continued - Page {page} of {pages}"`
- Aggregated page context:
  - `paginated_context={"tx.amount": "sum"}`
- Running footer text:
  - `footer_each="Page {page} of {pages}  |  Net Activity This Page: ${sum:tx.amount}"`
  - `footer_last="Page {page} of {pages}  |  Net Activity Total: ${total:tx.amount}"`

This makes page 1 visually distinct while keeping continuation pages compact and information-rich.

## CSS and Validation Behavior

Before final render, the example:
- Loads CSS in fixed layer order.
- Detects unscoped component selectors.
- Detects declarations that should be reviewed for compatibility.
- Performs component mount smoke validation and emits structured JSON diagnostics.

Strict mode:
- `FULLBLEED_VALIDATE_STRICT=1` promotes select warnings to hard failures.

## Useful Environment Flags

- `FULLBLEED_VALIDATE_STRICT=1`
- `FULLBLEED_EMIT_PAGE_DATA=1`
- `FULLBLEED_IMAGE_DPI=144`
- `FULLBLEED_DEBUG=1`
- `FULLBLEED_PERF=1`

PowerShell example:

```powershell
$env:FULLBLEED_VALIDATE_STRICT="1"
$env:FULLBLEED_IMAGE_DPI="180"
python report.py
```

## Customization Guide

Common edits:
- Replace statement data: `data/statement.csv`
- Update account/header layout: `components/header.py`
- Update summary and transaction table: `components/body.py`
- Adjust CSS layering:
  - `styles/tokens.css`
  - `components/styles/*.css`
  - `styles/report.css`
- Tune continuation-page behavior in `create_engine(...)`:
  - `page_margins`
  - `header_each`
  - `footer_each`
  - `footer_last`
  - `paginated_context`

## AI-Friendly Execution Checklist

1. Verify `data/statement.csv` is present and valid.
2. Run `python report.py`.
3. Confirm `output/bank_statement_component_mount_validation.json` has `"ok": true`.
4. Confirm `output/bank_statement.pdf` exists and page count is expected.
5. Use `output/bank_statement_page*.png` as the primary visual regression artifacts against `source.png`.
