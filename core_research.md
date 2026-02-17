# Core Performance Research: Internal PDF Linker, JIT, Math, and Dependency Posture

Date: 2026-02-17

## Scope Clarification

- This report treats "linker" as our internal PDF linker/finalization stage in `src/pdf.rs` (`PdfStreamWriter::finish`, `perf` span `pdf.link`), not the Rust/C toolchain linker.
- No code changes were made for this analysis. This is a research/backlog document only.

## North Star: Determinism (Hard Gate)

- Determinism is a non-negotiable constraint for every optimization in this document.
- No perf/compression/linker change is accepted if it introduces output variance for identical inputs/config.
- This applies to:
  - PDF bytes emitted by the internal linker.
  - PNG bytes emitted by native raster/finalized-PDF raster paths.
  - Parallel scheduling paths (`rayon`, batch, and chunked compression).

## Evidence Pack

- Build timing artifact:
  - `target/cargo-timings/cargo-timing-20260217T203757.4735055Z.html`
- Runtime perf logs:
  - `fullbleed_preflight_hot.log`
  - `examples/bank_statement/output/bank_statement.perf_hot.log`
  - `examples/bank_statement/output/bank_statement.jit.jsonl`
  - `examples/template-flagging-smoke/output/cli_image_compose.perf.jsonl`
- Primary code anchors:
  - `src/pdf.rs:318` (`PdfStreamWriter::finish`)
  - `src/pdf.rs:1119` (`write_object`)
  - `src/pdf.rs:1256` (`ensure_image`)
  - `src/pdf.rs:1321` (`ensure_form`)
  - `src/pdf.rs:2493` (`encode_stream_data`)
  - `src/pdf.rs:2604` (`ascii_hex_encode`)
  - `src/pdf.rs:3229` (`stream_object`)
  - `src/jit.rs:142` (`plan_document_with_overlay`)
  - `src/jit.rs:260` (`paint_plan`)
  - `src/jit.rs:290` (`paint_plan_parallel`)
  - `src/flowable.rs:117` (`TextLayoutCache`)
  - `src/flowable.rs:122` (`TextWidthCache`)
  - `src/html.rs:3012` (table style cache gating)
  - `src/pdf_raster.rs:519` (Form XObject parse path)
  - `src/pdf_raster.rs:558` (image data-uri cache)
  - `src/flate_native.rs:178` (LZ77 plan)
  - `src/flate_native.rs:353` (parallel deflate entrypoint)

## Baseline Snapshot

### Build (from source)

- `release` profile build timing page reports:
  - Total time: `202.1s`
  - Fresh units: `0`, dirty units: `246`
  - Max concurrency: `20`
- Largest compile units from timing report:
  - `lightningcss`: `95.4s` (`42.7s` codegen)
  - `fullbleed`: `48.3s`
  - `image`: `35.6s`
  - `lopdf`: `31.3s`
  - `ttf-parser 0.21.1`: `27.7s`
  - `tiny-skia`: `24.7s`
  - `rustybuzz`: `23.8s`

### Runtime spans and counters

- `fullbleed_preflight_hot.log`
  - `raster`: `149.246ms`
  - `css.parse`: `89.231ms`
  - `pdf.link`: `66.732ms`
  - `pdf.link.bytes`: `4,190,264`

- `examples/bank_statement/output/bank_statement.perf_hot.log`
  - `css.parse`: `146.063ms`
  - `raster`: `104.561ms`
  - `pdf.link`: `40.191ms`
  - `pdf.link.bytes`: `1,811,609`
  - `pdf.link.pages`: `2`
  - `pdf.link.fonts`: `2`
  - `pdf.link.extgstates`: `1`
  - Cache signals:
    - `layout.tablecell.width` hit ratio: `90.49%`
    - `layout.text.width` hit ratio: `54.95%`
    - `story.table.row_style_cache` hit ratio: `0%`
    - `story.table.cell_style_cache` hit ratio: `0%`

- `examples/bank_statement/output/bank_statement.jit.jsonl`
  - `jit.metrics` mode: `"off"`
  - `jit.link.ms`: `40.191`
  - `DECLARATION_PARSED_NO_EFFECT` count: `186`

- `examples/template-flagging-smoke/output/cli_image_compose.perf.jsonl`
  - `raster.finalized_pdf`: `47.603ms`
  - `raster.finalized_pdf.pages`: `2`
  - `pdf.link`: `0.044ms` (very small output in this run)

## Internal PDF Linker Analysis (Primary Focus)

### What "our linker" is doing now

- Finalization/link stage lives in `src/pdf.rs:318` and performs, in order:
  - Font object flush
  - Global resources dictionary build
  - Page tree and pages root
  - Optional tagged-PDF structure tree
  - Compliance objects and catalog
  - XRef/trailer emission
  - Perf logging (`pdf.link`) and counters

### Main structural costs

1. String-heavy object assembly in hot path
- Extensive `format!` and `join` during resource/page/catalog/xref assembly:
  - `src/pdf.rs:438`, `src/pdf.rs:463`, `src/pdf.rs:470`, `src/pdf.rs:491`, `src/pdf.rs:672`, `src/pdf.rs:684`
- Every object header uses `format!` in `write_pdf_object`:
  - `src/pdf.rs:3305`, `src/pdf.rs:3315`

2. ASCIIHex stream encoding inflates bytes and CPU
- Image/font/ICC streams are ASCIIHex-wrapped:
  - `src/pdf.rs:2493`, `src/pdf.rs:2553`, `src/pdf.rs:2572`, `src/pdf.rs:2604`
- This roughly doubles stream payload size before PDF write, plus hex formatting overhead.

3. Page/form content streams are not flate-compressed
- `stream_object` emits `<< /Length ... >>` only, no filter:
  - `src/pdf.rs:3229`
- So page content and form command streams stay raw text, increasing `pdf.link.bytes` and write bandwidth.

4. Dedupe exists but is scoped
- Image/form dedupe is present and useful (`reuse_xobjects`):
  - `src/pdf.rs:1293`, `src/pdf.rs:1334`
- This helps repeated resources, but does not address core object emission overhead.

### Interpretation of linker baseline

- On current workloads, `pdf.link` is material but not always top:
  - `66.732ms` (preflight)
  - `40.191ms` (bank statement)
- The byte volume (`1.8MB` to `4.2MB` in sampled logs) indicates stream representation (raw + hex) is a primary lever.

## JIT and Layout Findings

1. Command cloning is widespread in planning/replay
- JIT plan copies page/background/overlay commands into paintables:
  - `src/jit.rs:170`, `src/jit.rs:186`, `src/jit.rs:204`
- Paint stage clones command vectors again:
  - `src/jit.rs:270`, `src/jit.rs:304`
- Additional clone points in library merge paths:
  - `src/lib.rs:985`, `src/lib.rs:1206`

2. JIT mode in measured workload is off
- `bank_statement.jit.jsonl` reports mode `"off"`, so current observed hot path is mostly CSS/layout/raster/link.
- JIT optimization still matters for plan/replay and batch paths where cloning cost scales with pages.

3. Table style caches are effectively disabled in common selector conditions
- Cache gating depends on selector complexity and sibling/positional selectors:
  - `src/html.rs:2896`, `src/html.rs:3012`, `src/html.rs:3103`
- In measured workload, row/cell style cache hit ratios are `0%`, consistent with these gates.

4. Width/layout caches use linear scan vectors
- `TextWidthCache` and `TextLayoutCache` are Vec-backed linear lookups:
  - `src/flowable.rs:126`, `src/flowable.rs:145`
- Works for very small caps, but becomes lock+scan overhead under heavy repeated text.

## Mathematical and Algorithmic Findings

1. Native deflate is already compressed-block, fixed-Huffman, parallel planned
- Entrypoint and architecture:
  - `src/flate_native.rs:353`
  - Chunk plan in parallel (`par_iter`), serial bitstream assembly
  - LZ77 with hash chains:
    - `src/flate_native.rs:178`

2. Remaining compression opportunities are algorithmic, not just plumbing
- Fixed Huffman is deterministic and simple, but leaves ratio on mixed-entropy content.
- No cross-chunk match continuity currently (chunk-local windows).
- Match search is byte-wise; SIMD/prefetch-friendly approaches remain open.

3. Finalized-PDF raster path currently pays extra encode/decode transformations
- XObject images converted to data URIs (`base64`) in parser:
  - `src/pdf_raster.rs:558`, `src/pdf_raster.rs:956`
- Raster loader decodes those sources again in image path:
  - `src/raster.rs:347`
- Form XObject parse path is recursive per use:
  - `src/pdf_raster.rs:519`

## Dependency and System-Dependency Posture

### Current posture

- No `libz-sys`/`zlib` linkage was found in lockfile.
- `flate2` remains transitive (`Cargo.lock:531`) via `lopdf`/`png` stacks, but lock shows `miniz_oxide` backend, not system zlib.
- `-sys` crates present in lock:
  - `core-foundation-sys` (`Cargo.lock:288`)
  - `js-sys` (`Cargo.lock:824`)
- These are transitive/target-specific and not evidence of runtime system zlib dependency on this Windows build path.

### Compile-surface opportunities

- Duplicate crate families materially increase compile cost:
  - `syn` v1 + v2
  - `png` v0.17 + v0.18
  - `ttf-parser` v0.20 + v0.21
  - `cssparser` v0.27 + v0.33
- Top compile-time cost remains CSS stack (`lightningcss`) and image/pdf stack.

## Prioritized Backlog (Breadcrumb + Implementation Theory)

- Global rule for all backlog items:
  - Preserve stable object ordering, stable iteration ordering, and stable tie-break behavior.
  - Any heuristic must include deterministic tie resolution.

### P0-1: Binary Stream Writer Path in Internal PDF Linker

- Breadcrumb:
  - `src/pdf.rs:2493`
  - `src/pdf.rs:2604`
  - `src/pdf.rs:3305`
- Implementation theory:
  - Introduce byte-oriented stream object writer (direct binary stream payloads), avoiding ASCIIHex where not required.
  - Keep deterministic object ordering and lengths by calculating length from byte slices before write.
- Expected impact:
  - Lower `pdf.link.bytes`, less formatting CPU, lower `pdf.link` ms.
- Validation:
  - Compare `pdf.link.bytes` and `pdf.link` span on preflight and bank_statement logs.

### P0-2: Flate-Compress Page/Form Content Streams in Linker

- Breadcrumb:
  - `src/pdf.rs:1321`
  - `src/pdf.rs:3229`
  - `src/flate_native.rs:353`
- Implementation theory:
  - Apply native flate to page content and form command streams with `/Filter /FlateDecode`.
  - Preserve small-stream bypass threshold to avoid overhead on tiny pages.
- Expected impact:
  - Significant byte reduction and I/O savings, likely better end-to-end write time.
- Validation:
  - Track `pdf.link.bytes`, PDF output size, and visual regressions.

### P0-3: Linker Object Assembly Arena/Buffer Strategy

- Breadcrumb:
  - `src/pdf.rs:438`
  - `src/pdf.rs:470`
  - `src/pdf.rs:684`
  - `src/pdf.rs:3315`
- Implementation theory:
  - Replace repeated `format!`/`join` churn in hot loops with pre-sized `String`/`Vec<u8>` builders and append helpers.
  - Move static fragments to constants and reduce temporary allocations.
- Expected impact:
  - Lower allocator pressure in `pdf.link`.
- Validation:
  - Allocation profiling and `pdf.link` span deltas.

### P0-4: Finalized-PDF Raster Binary Image Cache (Remove Base64 Churn)

- Breadcrumb:
  - `src/pdf_raster.rs:558`
  - `src/pdf_raster.rs:733`
  - `src/pdf_raster.rs:956`
  - `src/raster.rs:347`
- Implementation theory:
  - Cache decoded image bytes/pixmaps keyed by object id, and pass binary handles instead of data URI strings.
  - Avoid encode/decode ping-pong through base64.
- Expected impact:
  - Lower `raster.finalized_pdf` ms and memory churn on compose workloads.
- Validation:
  - Compose smoke perf file (`raster.finalized_pdf`) on multi-page docs.

### P1-5: JIT Zero-Copy Command Referencing

- Breadcrumb:
  - `src/jit.rs:170`
  - `src/jit.rs:270`
  - `src/jit.rs:304`
- Implementation theory:
  - Store command slices or arc-backed command blocks in `Paintable` instead of cloned `Vec<Command>` at each stage.
  - Replay by reference with stable page ordering.
- Expected impact:
  - Reduced memory traffic in plan/replay and batch parallel modes.
- Validation:
  - Throughput and peak RSS in `JitMode::PlanAndReplay`.

### P1-6: Table Style Cache Enablement Under Selector Complexity

- Breadcrumb:
  - `src/html.rs:2896`
  - `src/html.rs:3012`
  - `src/html.rs:3103`
- Implementation theory:
  - Introduce cache-keyed style reuse (structural signature + selector epoch) so cache can stay safe even with sibling/positional selectors.
  - Keep fallback correctness path when selector dependence is truly row-specific.
- Expected impact:
  - Improve row/cell style cache hit rates from current `0%` in affected workloads.
- Validation:
  - `story.table.row_style_cache_hit` and `story.table.cell_style_cache_hit` counters.

### P1-7: Replace Linear Text Caches with Hash + Ring Eviction

- Breadcrumb:
  - `src/flowable.rs:126`
  - `src/flowable.rs:145`
- Implementation theory:
  - Replace Vec scan caches with hash-indexed lookup plus deterministic ring eviction.
  - Keep small bounded footprint to preserve determinism and memory control.
- Expected impact:
  - Lower lock-hold and lookup cost in text-heavy layout loops.
- Validation:
  - `layout.text.width` and `layout.tablecell.width` cache-hit latency trends.

### P1-8: Native Deflate Dynamic-Huffman Mode with Deterministic Block Heuristics

- Breadcrumb:
  - `src/flate_native.rs:317`
  - `src/flate_native.rs:353`
- Implementation theory:
  - Add per-block symbol histogram and deterministic dynamic-Huffman fallback when estimated bit cost beats fixed-Huffman.
  - Keep deterministic tie-break rules for reproducible output.
- Expected impact:
  - Better compression ratio on mixed text/image content.
- Validation:
  - Size benchmarks across representative documents; decode roundtrip and determinism checks.

### P1-9: Cross-Chunk Match Continuity in Parallel Deflate

- Breadcrumb:
  - `src/flate_native.rs:137`
  - `src/flate_native.rs:178`
  - `src/flate_native.rs:356`
- Implementation theory:
  - Add optional rolling dictionary handoff between chunk boundaries while retaining parallel planning.
  - Keep deterministic chunk schedule and merge order.
- Expected impact:
  - Recovers ratio lost at chunk boundaries in repetitive payloads.
- Validation:
  - Compression ratio delta on large repetitive/mixed corpora.

### P2-10: Compile-Time Dependency Dedupe and Feature Pruning

- Breadcrumb:
  - `Cargo.toml`
  - `target/cargo-timings/cargo-timing-20260217T203757.4735055Z.html`
- Implementation theory:
  - Reduce duplicated major crate lines where possible and gate heavyweight features by default profile/feature sets.
  - Keep runtime behavior unchanged; scope this to build-time/perf of development loops.
- Expected impact:
  - Lower clean build time and CI cost.
- Validation:
  - Re-run `cargo build --release --timings` and compare total time/top units.

## Recommended Next Slice

1. Add/lock determinism regression gates first (listed below) so perf work cannot drift output stability.
2. Execute `P0-1` and `P0-2` together (binary stream path + page/form flate). They are the highest-confidence wins for our internal PDF linker.
3. Execute `P0-4` next for compose finalized-PDF image throughput.
4. Follow with `P1-5` and `P1-6` for JIT/layout scalability.

## Determinism Regression Gates (Must Pass Before Merge)

1. PDF byte determinism:
   - Same input, same config, repeated runs must produce byte-identical PDFs.
2. Parallel determinism:
   - Vary thread counts (`RAYON_NUM_THREADS=1` vs higher) and confirm byte-identical PDFs/PNGs.
3. Raster determinism:
   - `render_image_pages` and `render_finalized_pdf_image_pages` produce byte-identical PNGs across repeated runs.
4. Compression determinism:
   - Native deflate output is byte-identical across repeated runs and thread counts for same payload.
5. Ordering determinism:
   - Object/resource ordering in linker output remains stable regardless of hash map insertion timing.
