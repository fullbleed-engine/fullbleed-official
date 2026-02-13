# Coastal Menu (Python UI)

This example rebuilds `costal_menu.png` using the component-style Python UI layer.

## Run

```powershell
cd examples/coastal_menu
python report.py
```

Outputs:
- `output/coastal_menu.pdf`
- `output/coastal_menu_page1.png` (if PyMuPDF is available)

## Structure

- `components/fb_ui.py`: minimal component runtime (`@component`, `@Document`, `el`, compiler)
- `components/primitives.py`: typed layout primitives (`Stack`, `Row`, `Text`, `PriceRow`, `IconLabel`)
- `components/left_panel.py`: contact rail
- `components/center_panel.py`: starters/mains column
- `components/right_panel.py`: hero + beverages rail
- `styles/tokens.py`: shared design tokens surfaced into CSS variables
- `styles/coastal_menu.css`: page-level visual treatment (token-backed)
- `pipeline/validation.py`: render-time validation helper with structured diagnostics
- `report.py`: data + orchestration + render call

## Why this is useful

- Data and layout are separated.
- Reusable section components make parity iteration fast.
- Small typed primitives reduce stringly-typed HTML authoring mistakes.
- Validation JSON enables agent loops without reading binary artifacts first.
- The renderer input stays canonical: HTML + CSS + bundled assets.
