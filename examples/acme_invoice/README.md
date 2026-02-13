# Acme Invoice Example

This example shows a production-style, component-driven invoice built with Fullbleed's Python API and rendered from CSV data.

It is designed to be useful for:
- Human authors who want a clean starting point for transactional PDFs.
- AI agents that need a deterministic, scriptable pipeline with validation artifacts.

## What This Example Demonstrates

- CSV-driven document assembly (`data/invoice.csv`).
- Component composition with reusable sections:
  - `components/header.py`
  - `components/body.py`
  - `components/footer.py`
- Layered CSS loading with explicit order and validation (`CSS_LAYER_ORDER` in `report.py`).
- Component mount smoke validation before final render (`validate_component_mount`).
- Optional diagnostics and metadata outputs (JIT, perf, page data).

## Image-to-PDF Workflow (First-Class)

This example is intended to be built the same way many real documents are built: start from a target visual and iteratively match it.

Recommended loop:
- Choose a target mock/screenshot for the invoice look.
- Map visual regions directly to `Header`, `Body`, and `Footer` components.
- Run `python report.py` after each style or structure edit.
- Compare the target image against `output/acme_sample_invoice_page1.png`.
- Raise preview DPI during final polish:
  - `FULLBLEED_IMAGE_DPI=180` or higher.
- Keep validation in lockstep with visuals:
  - `output/acme_sample_invoice_component_mount_validation.json`
  - `output/acme_sample_invoice_css_layers.json`

For AI-driven iteration, treat the target image as the spec and the component tree as the editable structure that converges to that spec.

## Directory Layout

```text
examples/acme_invoice/
|- report.py
|- data/invoice.csv
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

Note: `templates/invoice.css` exists for reference but is not part of the active component render path in `report.py`.

## Data Contract (`data/invoice.csv`)

CSV header:

```csv
row_type,key,value,description,qty,rate
```

Supported `row_type` values:
- `meta`: key/value document metadata.
- `item`: invoice line items (`description`, `qty`, `rate`).

Required `meta` keys enforced by `load_invoice_data(...)`:
- `studio_name`
- `studio_tagline`
- `from_company`
- `from_email`
- `from_website`
- `from_address_line1`
- `from_address_line2`
- `bill_company`
- `bill_contact`
- `bill_email`
- `bill_address_line1`
- `bill_address_line2`
- `invoice_number`
- `invoice_date`

Optional `meta`:
- `tax_rate` (defaults to `0.08` if omitted)

## Run

From repository root:

```powershell
cd examples/acme_invoice
python report.py
```

## Render Outputs

Generated artifacts:
- `output/acme_sample_invoice.pdf`
- `output/acme_sample_invoice_page1.png`
- `output/acme_sample_invoice_component_mount_validation.json`
- `output/acme_sample_invoice_css_layers.json`

Optional artifacts (env-controlled):
- `output/acme_sample_invoice_page_data.json` (`FULLBLEED_EMIT_PAGE_DATA=1`)
- `output/acme_sample_invoice.jit.jsonl` (`FULLBLEED_DEBUG=1`)
- `output/acme_sample_invoice.perf.jsonl` (`FULLBLEED_PERF=1`)

## Engine Usage (Specific to This Example)

`report.py` configures:
- `PdfEngine(page_width="8.5in", page_height="11in", margin="0in")`
- Asset bundle registration for vendored Bootstrap, Inter, and Bootstrap Icons.
- Final PDF rendering via:
  - `render_pdf_to_file(...)`, or
  - `render_pdf_with_page_data(...)` when page data emission is enabled.
- Preview PNG generation via engine image APIs when available.

## CSS and Validation Behavior

Before render, this example:
- Loads CSS layers in fixed order.
- Scans component CSS for unscoped selectors.
- Scans component CSS for declarations known to be poor fit for this pipeline.
- Performs a component mount dry-run validation and writes a JSON report.

Strict mode:
- `FULLBLEED_VALIDATE_STRICT=1` converts validation warnings into blocking failures for CI-style enforcement.

## Useful Environment Flags

- `FULLBLEED_VALIDATE_STRICT=1`: fail on strict validation issues.
- `FULLBLEED_EMIT_PAGE_DATA=1`: write page data JSON.
- `FULLBLEED_IMAGE_DPI=144`: set preview PNG DPI.
- `FULLBLEED_DEBUG=1`: emit JIT debug log.
- `FULLBLEED_PERF=1`: emit perf log.

PowerShell example:

```powershell
$env:FULLBLEED_VALIDATE_STRICT="1"
$env:FULLBLEED_EMIT_PAGE_DATA="1"
python report.py
```

## Customization Guide

Typical edits:
- Change invoice data: `data/invoice.csv`
- Change section markup: `components/header.py`, `components/body.py`, `components/footer.py`
- Change reusable building blocks: `components/primitives.py`
- Change styles by layer:
  - tokens: `styles/tokens.css`
  - component styles: `components/styles/*.css`
  - final composition: `styles/report.css`

## AI-Friendly Execution Checklist

1. Ensure vendored assets exist under `vendor/`.
2. Run `python report.py`.
3. Confirm `output/acme_sample_invoice_component_mount_validation.json` has `"ok": true`.
4. Confirm `output/acme_sample_invoice.pdf` is non-empty.
5. Use `output/acme_sample_invoice_page1.png` as the primary visual regression artifact against your target image.
