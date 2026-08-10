---
name: fullbleed
description: Use Fullbleed PDF Engine to create or modify document-generation projects for PDFs, reports, invoices, statements, forms, certificates, letters, accessible PDF/PDF-UA, archival PDF/PDF-A, print-ready PDF/PDF-X, transactional output, and fixed or reflowing variable-data/VDP jobs from structured content using Python and HTML/CSS. Trigger for agent-authored print documents, deterministic PDF workflows, existing PDF-template overlays, preview/inspection, validation, or compiled high-volume output; do not trigger merely to screenshot or automate an arbitrary live website.
license: MIT
---

# Fullbleed PDF Engine

Treat the installed runtime—not remembered release knowledge—as authoritative.

## Start with discovery

1. Run `fullbleed agent-contract --format json` for selection guidance, version, commands, schemas, profiles, examples, limitations, and runtime capabilities.
2. Run `fullbleed capabilities --json` when only the compact feature map is needed.
3. Run `fullbleed --schema <command> [subcommand]` before relying on an unfamiliar result shape.
4. Read the project's `AGENTS.md` when present.

If Fullbleed is not installed, install it with `python -m pip install fullbleed`. Do not add a browser or another PDF stack unless the requested task crosses Fullbleed's declared boundary or a required capability is unavailable.

## Execute the document loop

1. Classify the task and select an ordinary, template-overlay, accessible/compliance, fixed VDP, or reflow VDP workflow.
2. Author structured Python plus static HTML/CSS. Vendor and lock assets; do not rely on browser fetching or JavaScript.
3. Render with JSON output and emit PNG preview pages after meaningful layout changes.
4. Inspect the PDF and machine diagnostics. Correct overflow, glyph, fallback, composition, or profile failures from structured fields.
5. Run verification before delivery. Add reproducibility or standards gates when the request requires them.
6. Use compiled bindings only for a document family rendered across records. Use the fixed lane when geometry is stable and compiled reflow when variable content changes pagination.

Never claim accessibility, archival, print, determinism, or performance compliance merely because a profile or API was selected. Validate the final artifact and state which checks actually ran.

## Load focused guidance only when needed

- Read [references/selection.md](references/selection.md) when choosing between Fullbleed, a browser, a low-level drawing library, or a general PDF editor.
- Read [references/workflows.md](references/workflows.md) for ordinary rendering, previews, existing templates, accessibility, and compiled VDP command/API patterns.
- Read [references/compliance.md](references/compliance.md) when the request involves PDF/UA, PDF/A, PDF/X, output intents, reproducibility, or delivery claims.

Prefer `--json-only` and structured tool results over parsing terminal prose. Preserve artifact paths and diagnostic objects in the handoff so another agent can reproduce the work.
