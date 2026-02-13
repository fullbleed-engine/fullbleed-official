# Engine Architecture

This document describes the Rust engine layer in `src/` and how it maps to the Python and CLI interfaces.

## Core modules

- `src/lib.rs`: top-level engine types (`FullBleed`, `FullBleedBuilder`, export surface)
- `src/html.rs`: HTML parsing and block conversion
- `src/style.rs`: CSS parsing and style resolution
- `src/flowable.rs`: layout primitives used by pagination
- `src/frame.rs`: frame placement and splitting behavior
- `src/doc_template.rs`: page creation and flowable placement loop
- `src/page_template.rs`: per-page template/frame definitions
- `src/pdf.rs`: PDF serialization options and profiles
- `src/python.rs`: Python bindings for `PdfEngine`, assets, and helpers

## Render pipeline

At a high level:

1. Build a `FullBleed` engine from `FullBleedBuilder`.
2. Parse HTML + CSS.
3. Resolve computed styles and generate flowables.
4. Build pages through `DocTemplate` and frame placement.
5. Apply headers, footers, watermark, and optional page-data context substitutions.
6. Serialize to PDF bytes or file.

## Pagination and per-page template model

`DocTemplate` uses a `Vec<PageTemplate>` and selects templates by page index with this rule:

- Page 1 uses template index `0`
- Page 2 uses template index `1` (if present)
- ...
- Remaining pages reuse the last template

That behavior is the basis for per-page templating in long reports.

## Assets and font handling

The engine supports bundle assets via `AssetBundle`:

- `css`
- `font` (`.ttf`, `.otf`)
- `image`
- `svg`
- `other`

Fonts are validated during registration. Asset bundle CSS is merged into the render CSS input.

## Diagnostics and validation signals

The engine and CLI expose validation signals used by preflight workflows:

- Glyph coverage report (`render_pdf_with_glyph_report`)
- Paginated page data (`render_pdf_with_page_data`)
- JIT logs (`jit_mode`, debug log output)
- Perf logs (timing spans and summaries)

These are consumed by CLI `--fail-on` policies and repro workflows.

## PDF output options

Engine options include:

- `pdf_version`: `1.7` or `2.0`
- `pdf_profile`: `none`, `pdfa2b`, `pdfx4`, `tagged`
- `color_space`: `rgb` or `cmyk`
- output intent ICC embedding and metadata fields

## Watermark model

Watermark supports:

- kind: text/html/image
- layer: background or overlay
- semantics: visual/artifact/ocg
- opacity/rotation/font options

## Threading and parallel render

Batch APIs include parallel methods. Python bindings release the GIL around long render operations.

