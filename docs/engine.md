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
- `pdf_profile`: `none`, `pdfa1a`, `pdfa1b`, `pdfa2a`, `pdfa2b`, `pdfa2u`, `pdfa3a`, `pdfa3b`, `pdfa3u`, `pdfa4`, `pdfa4e`, `pdfa4f`, `pdfx4`, `pdfua1`, `pdfua2`, `pdfvt1`, `wtpdf1r`, `wtpdf1a`, `tagged`
- `color_space`: `rgb` or `cmyk`
- output intent ICC embedding and metadata fields

PDF/A and PDF/VT/X profiles require an output intent. PDF/A, PDF/X/VT, PDF/UA,
and WTPDF profiles require embedded fonts when text is used. `pdfa4`, `pdfa4e`, `pdfa4f`, `pdfua2`, `wtpdf1r`, and `wtpdf1a` emit PDF 2.0 automatically. `pdfua1`,
`pdfua2`, `wtpdf1r`, and `wtpdf1a` enable the tagged-PDF structure path and emit identification
metadata. Use the accessibility verifier/seed traces as the machine gate before
making external conformance claims.

Use `python tools/validate_pdf_profiles.py --download-verapdf --install-pdf-oxide --strict-external`
for the profile conformance gate. It regenerates canonical profile specimens,
records inspect/JIT evidence, verifies byte-for-byte replay determinism, runs
veraPDF for PDF/A/PDF/UA/WTPDF profiles, and runs PDF/X-4 validation for `pdfx4` and
the PDF/X-4 base of `pdfvt1`. PDF/A-4f specimens also emit and inspect a
deterministic associated-file `EmbeddedFiles` name tree. WTPDF specimens emit and inspect PDF Declaration evidence. PDF/VT specimens also emit and inspect a minimal
document-part graph (`DPartRoot`, `DPartRootNode`, one-level `NodeNameList`,
leaf page range, and page `/DPart` references), plus a supplemental multipage
specimen that proves `/Start` and `/End`, reported as granular booleans plus
the aggregate `pdfvt_dpart_graph_valid` gate.
Pass `--pdfvt-cmd "tool --input {pdf}" --require-dedicated-pdfvt` to make a
dedicated PDF/VT preflight tool part of the same gate.

The distributed Python wheel feature set is `python,svg_raster`, matching the
CLI capability claim for SVG raster fallback. Source builds that omit
`svg_raster` keep native vector SVG support, but unsupported SVG features cannot
fall back to raster output.
`fullbleed capabilities --json` reports the compiled `svg_raster` build feature
and groups SVG support into native-vector, raster-fallback-required, and
unsupported/known-loss feature lists.

## Watermark model

Watermark supports:

- kind: text/html/image
- layer: background or overlay
- semantics: visual/artifact/ocg
- opacity/rotation/font options

## Threading and parallel render

Batch APIs include parallel methods. Python bindings release the GIL around long render operations.
