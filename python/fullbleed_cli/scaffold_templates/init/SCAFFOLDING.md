# Fullbleed Scaffolding Guide

This scaffold is intentionally component-driven.

Use `report.py` as the composition root and keep rendering logic predictable:
- Load and normalize data.
- Compose page-level sections.
- Render and validate.

## Starter Structure

The default `Header`, `Body`, and `Footer` files are a fast starting point, not a required architecture.

Use them as:
- Initial section slots for rapid implementation.
- A place to validate style system, assets, and data flow quickly.

As your document grows, rename or split by domain intent (for example `hero`, `line_items`, `totals`, `notes`, `sidebar`, `payment_terms`).

## Component Granularity Guidelines

Prefer components that are meaningful document sections, not tiny wrappers.

Good reasons to split a component:
- The section is reused.
- The section has distinct data contracts.
- The section exceeds comfortable readability.
- The section needs isolated styling and visual iteration.

Good reasons to keep components merged:
- The logic is simple and highly local.
- Splitting would add indirection without reuse.

Rule of thumb:
- Start with a few clear section components.
- Extract smaller pieces only when reuse or clarity improves.

## Styling Model

Keep styling layered and intentional:
1. `styles/tokens.css` (design tokens + page-level defaults)
2. `components/styles/primitives.css` (primitive utility styles)
3. `components/styles/*.css` (section styles)
4. `styles/report.css` (final composition/page rules)

Always scope selectors to the scaffold root (`[data-fb-role="document-root"]`) in component CSS.

## Recommended Workflow

1. Update data mapping and composition in `report.py`.
2. Update section markup in `components/*.py`.
3. Update styles in `components/styles/*.css` and `styles/*.css`.
4. Run `python report.py`.
5. Inspect outputs in `output/`:
   - `report.pdf`
   - `report.html`
   - `report.css`
   - `report_page1.png`
   - `component_mount_validation.json`
   - `css_layers.json`

The scaffold now emits HTML/CSS via engine artifact emitters with document-level CSS metadata defaults
(`document_css_href`, `document_css_source_path`, `document_css_media`, `document_css_required`) so
linked artifact output is deterministic.

## Useful Diagnostics

Set environment flags when needed:
- `FULLBLEED_DEBUG=1`
- `FULLBLEED_PERF=1`
- `FULLBLEED_EMIT_PAGE_DATA=1`
- `FULLBLEED_IMAGE_DPI=144`
- `FULLBLEED_VALIDATE_STRICT=1`

## Keep It Practical

Aim for simple, explicit composition with reusable section components.

The scaffold is optimized so you can ship quickly first, then refactor safely as structure emerges.
