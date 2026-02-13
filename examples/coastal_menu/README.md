# Coastal Menu Example

This example is a style-forward restaurant menu built with composable Python components and Fullbleed rendering.

It is intended as a visual composition showcase for:
- Three-column full-bleed layout work.
- Component-scoped styling and reusable primitives.
- Token-driven theming from Python into CSS.
- Render validation output suitable for both human review and agent automation.

## What This Example Demonstrates

- Component sections arranged as a single layout shell:
  - `components/left_panel.py`
  - `components/center_panel.py`
  - `components/right_panel.py`
- Shared UI helpers and primitives:
  - `components/fb_ui.py`
  - `components/primitives.py`
- Python token injection into CSS:
  - `styles/tokens.py` -> `styles/coastal_menu.css`
- Validation pass before final render:
  - `pipeline/validation.py`
  - output written to `output/coastal_menu_validation.json`

## Directory Layout

```text
examples/coastal_menu/
|- report.py
|- costal_menu.png
|- components/
|  |- fb_ui.py
|  |- primitives.py
|  |- left_panel.py
|  |- center_panel.py
|  `- right_panel.py
|- styles/
|  |- tokens.py
|  `- coastal_menu.css
|- pipeline/
|  `- validation.py
|- vendor/
|  |- css/bootstrap.min.css
|  |- fonts/Inter-Variable.ttf
|  `- icons/bootstrap-icons.svg
`- output/
```

`costal_menu.png` is the source reference image used for visual parity checks.

## Run

From repository root:

```powershell
cd examples/coastal_menu
python report.py
```

## Render Outputs

Generated artifacts:
- `output/coastal_menu.pdf`
- `output/coastal_menu_page1.png`
- `output/coastal_menu_validation.json`

Console output includes:
- PDF byte size
- preview status
- validation status (`ok=True/False`)
- source reference path

## Engine Usage (Specific to This Example)

`report.py` configures:
- `PdfEngine(page_width="17in", page_height="8.5in", margin="0in", pdf_version="1.7")`
- Registered bundle assets (Bootstrap CSS, Inter font, Bootstrap Icons SVG)

The document component uses:
- `@Document(page="17x8.5-landscape", margin="0in", title=..., bootstrap=True, root_class="report-root")`

Important behavior:
- Engine geometry is controlled by `create_engine(...)`.
- `@Document(...)` provides document metadata/structure hints used by the component layer.

## Styling Model and Token Injection

`styles/coastal_menu.css` contains a token placeholder:

- `/* __TOKENS__ */`

At runtime:
- `render_root_vars()` from `styles/tokens.py` generates CSS custom properties.
- `load_css()` injects the generated token block into the stylesheet.

This enables theme tuning from Python without editing component markup.

## Validation Pipeline

`validate_render(...)` in `pipeline/validation.py` performs a preflight render check and returns:
- `ok`
- `bytes_written`
- `page_count`
- `checks`
- `diagnostics`

The serialized report is written to:
- `output/coastal_menu_validation.json`

Checks include:
- non-empty PDF buffer
- expected page count match
- glyph report sampling

## Preview Image Notes

`report.py` writes `output/coastal_menu_page1.png` using a PDF-to-image step.
If the preview dependency is unavailable, PDF output and validation still run.

## Customization Guide

Common edits:
- Left panel content and contact rows: `components/left_panel.py`
- Menu sections and items: `components/center_panel.py`
- Hero block and beverage section: `components/right_panel.py`
- Visual system and spacing: `styles/coastal_menu.css`
- Palette/type scale tokens: `styles/tokens.py`
- Input data currently lives in `report.py` helper functions:
  - `_left_rows()`
  - `_menu_sections()`
  - `_beverages()`

## AI-Friendly Execution Checklist

1. Run `python report.py`.
2. Confirm `output/coastal_menu_validation.json` reports `"ok": true`.
3. Confirm `output/coastal_menu.pdf` exists and is non-empty.
4. Use `output/coastal_menu_page1.png` for visual parity checks against `costal_menu.png`.
