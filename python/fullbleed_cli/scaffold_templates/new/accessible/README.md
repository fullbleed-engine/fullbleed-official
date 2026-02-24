# FullBleed Accessibility-First Scaffold

This starter is a component-first, accessibility-first document project using:

- `fullbleed.ui` for HTML primitives and document composition
- `fullbleed.ui.accessibility` for semantic wrappers and structural validation
- PDF/UA-targeted tagged PDF output (via `fullbleed.accessibility.AccessibilityEngine`)

## What You Get

- `report.py`: runnable example document with semantic table, field grid, and signature semantics
- `styles/report.css`: baseline print styles
- `output/`: generated artifacts (PDF, validation JSON, preview PNGs if supported)

## Run

```bash
python report.py
```

Generated files:

- `output/accessibility_scaffold.pdf`
- `output/accessibility_scaffold.html`
- `output/accessibility_scaffold_a11y_validation.json`
- `output/accessibility_scaffold_component_mount_validation.json`
- `output/accessibility_scaffold_a11y_verify_engine.json`
- `output/accessibility_scaffold_pmr_engine.json`
- `output/accessibility_scaffold_run_report.json`

## Customize

1. Replace the sample data in `report.py`.
2. Keep field/value content in `FieldGrid(FieldItem(...))` instead of generic layout rows.
3. Keep data tables in `SemanticTable*` wrappers.
4. Keep signature meaning in text (`SignatureBlock`) and treat visual marks as supplemental.
5. Run with `artifact.to_html(a11y_mode="raise")` during development to catch structural issues early.
