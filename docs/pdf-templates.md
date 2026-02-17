# PDF Templates and XObjects

This document defines the Fullbleed template-composition path for source PDF workflows.

## Scope

Fullbleed supports:
- HTML/CSS overlay rendering
- Rust finalize composition onto vendored PDF templates
- deterministic, page-aware template routing

Fullbleed does not support:
- AcroForm field filling/editing
- annotation editing
- signature workflows
- general-purpose PDF editing

Contract: templates are treated as vendored vector assets; data is applied by overlay composition.

## Public backend policy

- Public template composition path is Rust finalize.
- Use CLI `render --templates ... --template-binding ...` (auto-compose) or `finalize compose`.
- Do not rely on private/non-Rust finalize prototypes.

## Asset bundling policy for PDF templates

Use first-class PDF assets:
- `vendored_asset(path, "pdf")`
- `AssetBundle.add_file(path, "pdf", ...)`

Validation and metadata requirements:
- PDF bytes must parse
- encrypted PDFs are rejected
- metadata must be available (`pdf_version`, `page_count`, `encrypted`)

## Binding model

Template selection precedence:
1. `by_feature` match (`fb.feature.*`)
2. `by_page_template`
3. `default_template_id`

Recommended metadata marker:
- `data-fb="fb.feature.<name>=1"`

Blank marker support:
- feature metadata can be emitted from visually blank elements (for example `header`/`footer` markers).

## Canonical smoke fixtures

Minimal per-page template routing:
```bash
python examples/template-flagging-smoke/run_smoke.py
```

Templated back-page VDP routing:
```bash
python examples/template-flagging-smoke/run_vdp_backpage_smoke.py
```

The VDP back-page smoke also validates PDF asset registration/metadata behavior and invalid-PDF rejection.

## CI gates

Required template gates:
1. `examples/template-flagging-smoke/run_smoke.py`
2. `examples/template-flagging-smoke/run_vdp_backpage_smoke.py`

These are wired in `.github/workflows/ci.yml`.
