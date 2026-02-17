# Native Finalized-PDF Raster Sprint (Draft)

## Objective

- Deliver fully engine-native finalized-PDF raster output for compose/stamp image artifacts.
- Remove third-party runtime dependency paths from compose image emission.
- Tighten throughput on high-page composed jobs.
- Keep backend pure-Rust with no system-library raster/decompression linkage.

## Scope Anchors

- `src/pdf_raster.rs`
- `src/lib.rs`
- `src/python.rs`
- `python/fullbleed_cli/cli.py`
- `examples/template-flagging-smoke/run_cli_compose_image_smoke.py`

## Current Status

1. Completed: native finalized-PDF raster backend added in Rust (`lopdf` parse + engine raster).
2. Completed: compose image emission now calls engine-native finalized-PDF raster APIs.
3. Completed: PyMuPDF fallback/dependency path removed from compose image workflow.
4. Completed: CI/smoke path validates compose image output without external raster dependency.
5. Completed: `flate2` backend pinned to Rust implementation in `Cargo.toml` (`default-features = false`, `rust_backend`).
6. Completed: direct PDF writer compression switched to in-house native encoder (`src/flate_native.rs`) with LZ77 + fixed-Huffman compressed blocks and parallel chunk planning, removing direct `flate2` usage from engine code.
7. In progress: parser/raster feature coverage expansion and perf caching.

## Implementation Theory (Breadcrumbed)

### A. Finalized PDF -> Engine Raster Bridge (P0)
- Breadcrumb:
  - `src/pdf_raster.rs::pdf_path_to_png_pages`
  - `src/pdf_raster.rs::parse_page`
- Theory:
  - Parse finalized PDF content streams/XObjects into engine commands and reuse `src/raster.rs` for PNG generation.
  - Preserve deterministic output and keep stack internal to engine.
- Delivered:
  - page-level parse with per-page media box handling
  - content operator support for compose-generated PDFs (`q/Q`, `cm`, path paint ops, `BT/Tf/Td/Tj`, `Do` Form/Image)

### B. Runtime API Surface (P0)
- Breadcrumb:
  - `src/lib.rs::render_finalized_pdf_image_pages`
  - `src/lib.rs::render_finalized_pdf_image_pages_to_dir`
  - `src/python.rs` bindings of same names
- Theory:
  - Expose finalized-PDF raster as first-class engine API to avoid Python-side raster dependencies.
- Delivered:
  - Rust + Python APIs landed and wired.

### C. CLI Compose Wiring (P0)
- Breadcrumb:
  - `python/fullbleed_cli/cli.py::_emit_image_artifacts_from_pdf`
  - `python/fullbleed_cli/cli.py::_render_with_template_compose`
- Theory:
  - In compose mode, emit PNGs from finalized output only; never rerender overlay HTML/CSS for artifact correctness.
- Delivered:
  - compose image mode now hard-routes to finalized-PDF native raster APIs.

### D. Validation + Regression Gate (P0)
- Breadcrumb:
  - `examples/template-flagging-smoke/run_cli_compose_image_smoke.py`
  - `.github/workflows/ci.yml`
- Theory:
  - Guard against regressions with explicit checks: `image_mode`, page-count parity, and template-background pixel evidence.
- Delivered:
  - smoke verifies compose-mode invariants without external image/PDF libs.

## Backlog (Discovered During Implementation)

1. Add rotated/sheared CTM image placement support in finalized-PDF parser path.
2. Expand image filter coverage beyond DCT/Flate-compatible paths (JPX/CCITT/etc.).
3. Improve text operator fidelity (`TJ` spacing precision, additional text-state ops).
4. Add decode/parse caching for repeated Form XObject streams on large composed documents.
5. Add perf benchmarks and CI thresholds specific to finalized-PDF raster spans.

## Acceptance Criteria

1. `render --templates ... --emit-image ...` succeeds without third-party raster dependencies.
2. Emitted PNGs are composed-PDF faithful for template-backed pages (validated by smoke).
3. Engine APIs expose finalized-PDF rasterization for direct Python/runtime use.
4. No regressions in existing Rust/Python smoke suites.
