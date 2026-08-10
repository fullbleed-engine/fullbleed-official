# Fullbleed Release Notes - 2.2.5

Status: release

Release date: August 10, 2026

## Summary

Fullbleed 2.2.5 hardens the compiled content-reflow lane introduced in 2.2.4. It fixes reported
fragmentation and named-string gaps, makes compiled-flow compression an explicit per-render API,
and brings Python capability discovery, documentation, examples, and golden artifacts into line
with the engine that ships in the wheel.

The authoritative layout and geometry representation remains signed Q32.32 fixed point. The
compiled lane reuses those values and re-executes the browser text-paint phase where alignment or
pixel snapping requires it; it does not replace layout geometry with floating-point estimates.

## 2.2.5 QA Hardening

- A long splittable table now uses the current page remainder after a kept heading instead of being
  deferred wholesale because the user-agent table style requests `break-inside: avoid`.
- An oversized authored `break-inside: avoid` box relaxes avoidance immediately when the complete
  box cannot fit a fresh fragmentainer, avoiding an underfilled heading-only page.
- Named strings accept both `content()` and `content(text)` in `string-set`; rules whose only
  declaration is `string-set` now survive style-delta filtering and populate continuation margin
  boxes through `content: string(name)`.
- `CompiledFlowCompression.Throughput` and `CompiledFlowCompression.Compact` select compression
  per render in Rust and Python. The worker encoder and linker fallback receive the same job-local
  policy, with no process-global environment state.
- `build_features()`, compiled-document stats, and `fullbleed capabilities --json` expose compiled
  reflow and its supported compression modes.
- The new `examples/compiled_reflow/` project demonstrates literal scalar bindings, escaped trusted
  structural rows, natural pagination, named running strings, and both compression policies.
- Performance documentation now distinguishes the independent hot median from the best sample and
  documents the structural-slot trust boundary. The built 2.2.5 wheel reproduces the established
  2.2.4 golden PDFs and page images byte-for-byte.

## Compiled Content Reflow

- Adds `CompiledDocument.render_pdf_reflow_bindings(...)` and
  `CompiledDocument.render_pdf_reflow_bindings_to_file(...)` to the stable Python ABI.
- Compiles literal text slots and explicit trusted structural slots declared with
  `data-fb-bind-html="slot"` from one recovered template DOM.
- Captures encountered page-count and content structures as guarded `CompiledFlowRecordProgram`
  variants. A guard miss performs ordinary fixed-point layout once and adds the new variant.
- Compiles eligible PDF pages into static vector segments plus dynamic text-paint slots.
- Lowers shaped glyph runs directly to PDF TJ operators, discovers glyphs in worker-local state,
  compresses page streams in native workers, and feeds one ordered bounded linker.
- Releases the Python GIL while native execution runs. The implementation uses Rust standard
  threads rather than Python multiprocessing.
- Keeps the fallback path explicit for unseen structures, unsupported resources, transformations,
  or values that violate every compiled geometry guard.

## Measured Variable-Reflow Performance

The final 2.2.4 Windows AMD64 `cp310-abi3` wheel was measured on an Intel Core i7-12700H with 20
logical processors. This remains the scoped performance baseline for 2.2.5; the QA hardening does
not replace it with an unmeasured claim. The independent workload contains 1,000 distinct records
and naturally produces 1,750 pages: 500/300/150/50 records produce 1/2/3/4 pages respectively.

- Independent follow-up measurement put the 29-sample hot direct-file median at 216.160 ms:
  **4,626.2 records/s and 8,095.8 pages/s**. The 168.995 ms best sample reached **5,917.3
  records/s and 10,355.3 pages/s**; the original 168.839 ms result is therefore best-case, not
  median performance.
- The first variant-discovery render took 0.509 s, or 3,436.8 pages/s. The fully cold job including
  setup, bindings, compilation, discovery, and output took 0.588 s, or about 2,975 pages/s.
- Every sample produced deterministic bytes. The throughput-tuned PDF is 6,870,320 bytes with
  SHA-256 `1dd02748af497ae2f477875ecf0ab87a6de5ceb55e097b4b775ddbf6d5038194`.
- The 2.2.4 artifact used `FULLBLEED_COMPILED_FLOW_DEFLATE_CHAIN=64` to select compact mode. The
  2.2.5 API exposes the same tradeoff per call as `CompiledFlowCompression.Compact`. That output is
  byte-for-byte identical to ordinary rendering: 5,298,961 bytes with SHA-256
  `bb3c441313a08fb00d3bd15f23a567981f3bbbd550908e3a9fe7a07ca5d7f138`.
- The independent verifier found all 24,900 markers once and in order, all 1,750 local page
  counters, and the exact expected page distribution.
- Final telemetry selected 20 workers and an 80-record bounded window. All 1,750 pages were
  worker-encoded with zero fallback or rejection pages.
- Exact TrueType subsetting remained active: 121 glyphs across two embedded subsets.

These measurements describe hot execution of compiled structural variants, not arbitrary unseen
HTML. Novel structure pays one fixed-point compile/layout pass before it joins the hot lane.

## Compiler and PDF Pipeline

- Stores resolved font identity in compiled paint patches and avoids repeated family
  normalization and registry lookups.
- Provides a direct ASCII numeric glyph shader while preserving the general shaping path.
- Reuses page-local Deflate workspaces and fuses page-program instantiation with compression.
- Uses a four-step deterministic Deflate search for throughput-sized compiled-flow streams by
  default. `CompiledFlowCompression.Compact` selects the 64-step search per job; streams smaller
  than 4 KiB retain that compact search in either mode.
- Preserves exact fixed-point alignment constraints, parent fit guards, text spacing, and CSS pixel
  snapping in each compiled patch.
- Keeps large style and flow objects heap-owned so the complete test matrix passes on the standard
  Windows thread stack.

## Compatibility

- Adds behavior without removing the 2.2.x Rust, Python, CLI, batch, fixed-copy, fixed-geometry
  binding, or Stable ABI surfaces.
- Retains `cp310-abi3` wheels for CPython 3.10 through 3.14 across the 15-platform release matrix.
- Retains Rust 1.85 as the minimum supported toolchain.
- Retains exact TrueType glyph closure, deterministic subset naming, and shared-resource PDF
  linking.

## Package Versions

- `fullbleed 2.2.5` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.3` on crates.io.
- `subsetter 0.2.6`, `rustc-hash 2.1.3`, and `unicode-bidi 0.3.18`, recorded in Cargo metadata and
  the generated CycloneDX SBOM.

## Verification Baseline

- Locked Rust default matrix: 1,185 passed, zero failed.
- Locked Rust `python,svg_raster` matrix: 1,185 passed, zero failed.
- Repository Python suite against the installed optimized ABI3 wheel: 261 passed, four skipped.
- Python Stable ABI compiled-document suite: 11 passed; focused ABI/CLI coverage: 60 passed.
- Three source-controlled golden cases verify, and all golden pages passed visual review.
- The independent table, oversized-avoid, and named-string probes pass against the packaged wheel.
- Independent reflow verification: 1,000 records, 1,750 pages, 24,900 markers, exact record order,
  and exact local page counters.
- Compact compiled output is byte-identical to ordinary rendering.
- Poppler raster inspection passed for a one-page record and both pages of the final continued
  record, with no clipping, overlap, missing header, or page-counter defect.
- The release workflow builds, installs, and smokes 15 `cp310-abi3` wheels plus one source
  distribution before publication.

The exact `v2.2.5` source is validated before registry publication. GitHub, PyPI, and crates.io are
published from that immutable tagged source, with the complete Python distribution set and its
SHA-256 manifest attached to the GitHub release.
