# Image + Perf Sprint (Draft)

## Problem Snapshot

- Symptom: native PNG output omits template/stamped page content in compose/stamp workflows.
- Companion sprint doc for this slice: `native_pdf_raster_sprint.md`.
- Current breadcrumb:
  - `python/fullbleed_cli/cli.py::_render_with_template_compose`
  - `python/fullbleed_cli/cli.py::_emit_image_artifacts`
  - `python/fullbleed_cli/cli.py` calls `engine.render_image_pages*`
  - `src/lib.rs::render_image_pages`
  - `src/raster.rs::document_to_png_pages`
- Why this happens: compose/stamp is done after overlay render (`src/finalize.rs`), but image artifacts are generated from pre-finalize HTML/CSS document state.
- Perf baseline signal (`fullbleed_preflight_hot.log`): `raster` is top span (~149ms), with additional duplicate render costs in artifact-heavy CLI paths.

## Sprint Goals

1. Make image artifacts represent final composed output in template/stamp mode.
2. Remove repeated render passes for artifact fan-out (PDF + page_data + glyph + image).
3. Reduce raster hot-path cost for text-heavy pages.

## Execution Status (Current)

1. Completed: compose-mode image emission now targets finalized composed PDF output path (CLI), not overlay rerender.
2. Completed: render outputs now include `image_mode` metadata (`overlay_document` or `composed_pdf`).
3. Completed: consolidated combined render APIs added for page data + glyph report (+ template bindings variant) to reduce duplicate passes.
4. Completed: dedicated compose-image smoke added (CLI path) with page-count parity and template-background pixel assertion.
5. Completed: native Rust finalized-PDF raster backend landed for compose image emission; PyMuPDF fallback path removed.
6. In progress: perf-specific gates and thresholds for compose-image workload.
7. In progress: compose-image timing instrumentation landed (`template_compose.image_emit_ms`); threshold policy still pending.
8. Completed: CI/runtime path no longer installs or depends on PyMuPDF for compose-image smoke.
9. Completed: pure-Rust compression/raster dependency posture enforced (`flate2` rust backend pinned; no `libz-sys`).
10. Completed: direct engine PDF stream compression moved to in-house native implementation (`src/flate_native.rs`) with deterministic LZ77 + fixed-Huffman deflate and parallel chunk planning/adler.
11. Completed: deterministic regression gates landed for PDF bytes, parallel thread-count variance, raster PNG bytes, and native deflate concurrency invariance.
12. Completed: cross-process CLI determinism harness landed (`examples/template-flagging-smoke/run_cli_determinism_smoke.py`) and is wired into CI goldens job.
13. Completed: internal PDF linker now Flate-compresses page/form command streams using native deterministic deflate with binary stream object emission.
14. Completed: linker regression tests now validate decoded page-content semantics under compressed streams and explicit compression-threshold behavior.
15. Completed: internal PDF linker now emits image/font/ICC streams as native binary streams (runtime writer path), removing ASCIIHex wrapper overhead.
16. Completed: `pdf.link` perf counts now include content-stream compression telemetry (`content_stream_raw_bytes`, `content_stream_encoded_bytes`, `content_stream_compressed`, `content_stream_ratio_ppm`).
17. Completed: removed legacy/dead PDF object-builder stream path (ASCIIHex/string stream builders) so runtime binary writer path is the single stream-emission implementation.
18. Completed: CLI deterministic hash contract extended to artifact-set digest when image artifacts are emitted (`fullbleed.artifact_digest.v1` over PDF hash + ordered image hashes), with output fields for `artifact_sha256`, `image_sha256`, and deterministic-hash mode/value.
19. Completed: engine Python API now supports optional deterministic hash-file emission (`deterministic_hash`) on core render methods (`render_pdf*` and batch variants), writing PDF SHA-256 to disk directly from API calls.
20. Completed: i9 permutation benchmark validated at `1000` records (single-chunk, parallel batch mode) with page parity and `ok=true` manifest contract; current measured wall time is ~`20.013s` for `4000` composed pages.
21. Completed: native raster font-style fidelity restored for image output fallback path:
   - style-aware system font candidate selection now honors bold/italic variants,
   - subset-prefixed PDF `BaseFont` names are normalized before replay (`ABCDEF+Family-Style`),
   - regression tests added for bold preference + subset normalization.

## Workstream A: Image Correctness (Stamped/Composed)

### 1. Repro Harness + Contract (P0)
- Breadcrumb:
  - `examples/template-flagging-smoke/run_smoke.py`
  - `examples/form-i9/report.py`
  - `python/fullbleed_cli/cli.py::cmd_render`
- Implementation theory:
  - Add a deterministic failing harness where template backgrounds are high-signal colors and overlay is sparse.
  - Assert emitted PNGs match composed page count and contain template background color signatures (not overlay-only white pages).
- Deliverables:
  - New smoke assertion script and JSON report fields.
  - CI hook in existing template smoke workflow.

### 2. Final-Image Source Decision Spike (P0)
- Breadcrumb:
  - `src/finalize.rs` (compose output producer)
  - `src/lib.rs`/`src/python.rs` (API surface)
  - `python/fullbleed_cli/cli.py::_emit_image_artifacts`
- Implementation theory:
  - We need a post-finalize image source for compose mode; pre-finalize document raster cannot represent template PDF content.
  - Spike and choose one path with constraints table (determinism, dependency footprint, runtime cost):
    - `A)` feature-gated finalized-PDF raster backend
    - `B)` restricted PDF replay path for finalize-produced PDFs
    - `C)` explicit dual-mode artifacts (`overlay` vs `composed`) with composed path behind optional backend
- Deliverable:
  - Written ADR section in this doc with selected path and rejection reasons.
 - Decision:
  - Selected `A`: native Rust finalized-PDF raster backend using existing engine stack (`lopdf` parse + engine raster), no third-party runtime renderer dependency.

### 3. Compose-Aware Image Emission (P0)
- Breadcrumb:
  - `python/fullbleed_cli/cli.py::_render_with_template_compose`
  - `python/fullbleed_cli/cli.py::_emit_image_artifacts`
  - `src/python.rs` + `src/lib.rs` (new API binding)
- Implementation theory:
  - Route `--emit-image` in compose mode through post-finalize image generation, not overlay re-render.
  - Keep current native overlay image APIs untouched for non-compose workflows.
- Deliverables:
  - New compose-aware image path.
  - Backward-compatible CLI behavior, plus explicit warning/error when composed image mode is requested but unavailable.

### 4. API + Docs Contract Update (P1)
- Breadcrumb:
  - `docs/cli.md`
  - `docs/python-api.md`
  - `README.md`
  - `cli_schema.md`
- Implementation theory:
  - Document artifact semantics clearly: overlay preview vs finalized composed image pages.
  - Avoid silent ambiguity in automation pipelines.
- Deliverables:
  - Updated docs and schema notes.

## Workstream B: Performance Tightening

### 5. Single-Pass Artifact Fan-Out (P0)
- Breadcrumb:
  - `python/fullbleed_cli/cli.py::_render_with_artifacts`
  - `src/lib.rs::render_with_page_data`, `render_with_glyph_report`, `render_image_pages`
  - `src/python.rs` bindings
- Implementation theory:
  - Introduce a consolidated render call that builds story/layout/plan once, then emits requested artifacts from the same in-memory document.
  - Remove branch paths that currently re-render when multiple artifact flags are set.
- Deliverables:
  - Unified artifact API path.
  - Verify single story/layout pass in perf logs.

### 6. Compose Path Render De-Dup (P0)
- Breadcrumb:
  - `python/fullbleed_cli/cli.py::_render_with_template_compose`
  - `src/lib.rs::render_with_page_data_and_template_bindings`
- Implementation theory:
  - Compose mode currently does overlay render, then optional glyph render, then optional image render.
  - Extend consolidated artifact path to include template bindings so compose can reuse one render result before finalize.
- Deliverables:
  - Compose path reduced to one overlay render + one finalize pass.

### 7. Raster Hot-Path Cache Plan (P1)
- Breadcrumb:
  - `src/raster.rs::draw_string`
  - `src/raster.rs::layout_text_glyphs*`
  - `src/raster.rs` font/glyph parsing flow
- Implementation theory:
  - Cache parsed font faces and reusable glyph outlines/placement by `(font_key, glyph_id, scale bucket)` to avoid repeated parse + outline work per draw.
  - Keep bounded memory via simple LRU/size cap.
- Deliverables:
  - Cache implementation with guardrails.
  - Bench delta on text-heavy sample.

### 8. Page-Parallel Rasterization (P1)
- Breadcrumb:
  - `src/raster.rs::document_to_png_pages`
- Implementation theory:
  - Raster pages independently in parallel (stable output ordering preserved), sharing read-mostly form/image caches.
  - Target throughput improvement for multi-page outputs at high DPI.
- Deliverables:
  - Parallel raster implementation.
  - Determinism check (byte-equal PNGs across runs).

### 9. Perf Gates + Budgets (P1)
- Breadcrumb:
  - `src/perf.rs`
  - `python/fullbleed_cli/cli.py` fail-on budget plumbing
  - smoke scripts in `examples/template-flagging-smoke/`
- Implementation theory:
  - Add explicit perf capture for compose-image path and set guardrails to catch regressions early.
  - Use top spans (`raster`, `css.parse`, `pdf.link`) as stable tracked metrics.
- Deliverables:
  - Baseline JSON artifact and CI threshold check.

## Acceptance Criteria

1. `render --templates ... --template-binding ... --emit-image ...` emits PNGs that visually/structurally match composed PDF page count and template backgrounds.
2. Artifact-heavy render paths run one story/layout pass per document.
3. No schema-breaking output changes without explicit version note.
4. Perf regression guard added for compose + image workflows.

## Proposed Execution Order

1. Item 1 (repro harness)
2. Item 2 (decision spike)
3. Items 3 + 4 (correctness implementation + contract)
4. Items 5 + 6 (render de-dup)
5. Items 7 + 8 (raster optimization)
6. Item 9 (perf gates)

## Open Questions For Review

1. Should composed image output be mandatory in default install, or feature-gated when a finalized-PDF raster backend is unavailable?
2. Do we want two explicit artifact modes (`overlay` and `composed`) to avoid changing existing expectations for current users?
3. What minimum perf gain target should gate merge for this sprint (for example 20% on template-compose + image workload)?

## Backlog (Discovered During Execution)

1. Add explicit render output metadata for image semantics (`image_mode: overlay|composed`) to prevent downstream ambiguity.
2. Completed: native Rust finalized-PDF raster path landed; compose image path no longer depends on Python-side PDF raster backends.
3. In progress: compose-image-specific timing now emitted (`template_compose.image_emit_ms`); CI thresholds and fail gates still to be defined.
4. Completed: dedicated CLI compose-image smoke test now fails on page-count/image-count divergence and template-background mismatch.
5. Completed: CLI JSON file readers now use BOM-tolerant decode (`utf-8-sig`) for file-backed JSON inputs.
6. Completed: compose image emission policy is native composed-PDF raster only (hard-fail if runtime lacks finalized PDF raster APIs).
7. New: extend native finalized-PDF raster support for additional PDF features (rotated/sheared image CTMs, JPX/CCITT image filters, richer text operators).
8. New: add coverage/perf cache for repeated Form XObject decode in finalized-PDF raster pass on high-page jobs.
9. Completed: in-house flate path upgraded from stored-block baseline to compressed blocks (native LZ77/Huffman) with parallel chunk scheduling.
10. New: add dynamic Huffman block mode and block-level entropy heuristics for better compression ratio on mixed-content assets.
11. New: improve cross-chunk match continuity (optional rolling dictionary handoff) while preserving deterministic parallel scheduling.
12. New: add cross-process determinism harness (fresh process runs) to complement in-process test determinism gates.
13. New: add deterministic artifact signature check in CI for representative HTML/CSS fixture set (PDF + PNG outputs).
14. Completed: CLI deterministic hash now emits artifact-set digest for image-enabled runs (while retaining PDF hash as `outputs.sha256`).
15. Completed: binary stream emission for image/font/ICC streams landed in runtime writer path.
16. Completed: linker compression counters landed in perf/debug outputs (`content_stream_raw_bytes`, `content_stream_encoded_bytes`, `content_stream_compressed`, `content_stream_ratio_ppm`).
17. Completed: legacy/dead object-builder stream path removed; runtime binary writer path is sole implementation.
18. New: add lint/CI guard against reintroducing alternate PDF stream encoding paths that bypass runtime binary writer.
19. Completed: engine API-level deterministic hash file parameter landed for render and batch methods.
20. New: consider engine-side artifact-set digest helper for image-emitting APIs (`render_image_pages*`) to match CLI hash semantics end-to-end.
21. New: add first-class i9 permutation runner support for exact deterministic record counts (for example `FULLBLEED_I9_RECORD_COUNT`) to avoid benchmark wrapper monkeypatching.
22. New: add checked-in i9 perf harness and trend baseline artifacts (runtime + pages/sec + output bytes for `100`, `1000`, `5000` records) to enable regression gating.
23. Completed: restore native raster font-style fidelity for image outputs:
   - style-aware system font fallback mapping added for bold/italic variants (`Helvetica-Bold`, `Times-BoldItalic`, etc.),
   - subset/style-encoded PDF `BaseFont` names now normalized before fallback lookup (`ABCDEF+Family-Style`),
   - regression coverage added for style-aware candidate selection and subset normalization.
