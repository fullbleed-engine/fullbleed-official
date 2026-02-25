# FullBleed Accessibility-First Scaffold

This starter is intentionally verbose. It is meant to demonstrate the current
FullBleed accessibility stack end-to-end, not just produce a minimal PDF.

It uses:

- `fullbleed.ui` for component/document composition
- `fullbleed.ui.accessibility` for semantic authoring primitives
- `fullbleed.accessibility.AccessibilityEngine` for PDF/UA-targeted bundle output
- engine-native verifier + PMR + PDF seed checks + non-visual trace artifacts

## What This Scaffold Demonstrates

Authoring primitives in `report.py`:

- landmarks: `Nav`, `Aside`, labeled `Region`
- structure: `Heading`, `Section`, `Details`, `Summary`
- semantic fields: `FieldGrid`, `FieldItem`
- semantic tables: `SemanticTable*`
- form semantics: `FieldSet`, `Legend`, `Label`, `HelpText`, `ErrorText`
- announcements: `Status`, `Alert`, `LiveRegion`
- signature semantics: `SignatureBlock` + decorative figure handling

Runtime/output behavior:

- HTML/CSS/PDF bundle emission through `AccessibilityEngine.render_bundle(...)`
- engine accessibility verifier (`fullbleed.a11y.verify.v1`)
- paged media ranker (`fullbleed.pmr.v1`)
- PDF/UA seed verifier (`fullbleed.pdf.ua_seed_verify.v1`)
- non-visual reading-order and structure traces (render-time + post-render)

## Run

```bash
python report.py
```

## Generated Artifacts

Core deliverables:

- `output/accessibility_scaffold.pdf`
- `output/accessibility_scaffold.html`
- `output/accessibility_scaffold.css`

Authoring validation:

- `output/accessibility_scaffold_a11y_validation.json`
- `output/accessibility_scaffold_component_mount_validation.json`
- `output/accessibility_scaffold_claim_evidence.json`

Engine audit artifacts:

- `output/accessibility_scaffold_a11y_verify_engine.json`
- `output/accessibility_scaffold_pmr_engine.json`
- `output/accessibility_scaffold_pdf_ua_seed_verify.json`

Non-visual PDF observability:

- `output/accessibility_scaffold_reading_order_trace.json` (post-render PDF parse seed trace)
- `output/accessibility_scaffold_reading_order_trace_render.json` (render-time trace)
- `output/accessibility_scaffold_pdf_structure_trace.json` (post-render PDF parse seed trace)
- `output/accessibility_scaffold_pdf_structure_trace_render.json` (render-time trace)

Run summary:

- `output/accessibility_scaffold_run_report.json`

## How To Work With It

1. Replace sample data in `report.py` with your document data.
2. Preserve semantics first:
   - use `FieldGrid(FieldItem(...))` for label/value content
   - use `SemanticTable*` for true data tables
   - keep signature meaning in text (`SignatureBlock`) and treat visual marks as supplemental
3. Keep remediation/process notes out of final CAV deliverables; record them in sidecars when needed.
4. Run `artifact.to_html(a11y_mode="raise")` during development to fail fast on authoring issues.
5. Use the emitted engine verifier/PMR/PDF seed artifacts and non-visual traces as CI/manual review inputs.

## Strictness During Iteration

- `AccessibilityEngine(strict=False)` is the default in this scaffold for feature iteration.
- You can set `strict=True` in `create_engine()` when testing fail-fast behavior.
- Authoring validation (`artifact.to_html(a11y_mode="raise")`) is already strict in this template.

## Font Vendoring

This scaffold vendors `Inter` in `vendor/fonts/Inter-Variable.ttf` and registers it
with an `AssetBundle` in `create_engine()`.

Do not rely on fallback core-font metrics for production templates. Fallbacks are
useful safety nets, but vendored fonts produce more stable spacing and better
repeatability.
