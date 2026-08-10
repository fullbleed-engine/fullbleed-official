# Fullbleed Release Notes - 2.2.4

Status: release

Release date: August 10, 2026

## Summary

Fullbleed 2.2.4 makes content-driven flow and pagination a first-class compiler target. A compiled
document now retains the recovered template DOM, binding contract, guarded structural flow
variants, fixed-point display programs, and reusable PDF page-paint programs. Once a structural
variant has been encountered, matching variable records bypass HTML parsing, selector matching,
layout, pagination, and static page serialization.

The authoritative layout and geometry representation remains signed Q32.32 fixed point. The
compiled lane reuses those values and re-executes the browser text-paint phase where alignment or
pixel snapping requires it; it does not replace layout geometry with floating-point estimates.

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

The final Windows AMD64 `cp310-abi3` wheel was measured on an Intel Core i7-12700H with 20 logical
processors. The independent workload contains 1,000 distinct records and naturally produces
1,750 pages: 500/300/150/50 records produce 1/2/3/4 pages respectively.

- Twenty-nine hot direct-file samples had a 168.839 ms median: **5,923 records/s and 10,365
  pages/s**. The best sample reached **12,510 pages/s**.
- The first compile-on-demand batch took 407.521 ms, or 4,294 pages/s.
- Every sample produced deterministic bytes. The throughput-tuned PDF is 6,870,320 bytes with
  SHA-256 `1dd02748af497ae2f477875ecf0ab87a6de5ceb55e097b4b775ddbf6d5038194`.
- `FULLBLEED_COMPILED_FLOW_DEFLATE_CHAIN=64` selects compact mode. That output is byte-for-byte
  identical to ordinary rendering: 5,298,961 bytes with SHA-256
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
- Uses a four-step deterministic Deflate search for throughput-sized compiled-flow streams.
  Streams smaller than 4 KiB retain the compact 64-step search for ordinary-output parity.
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

- `fullbleed 2.2.4` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.3` on crates.io.
- `subsetter 0.2.6`, `rustc-hash 2.1.3`, and `unicode-bidi 0.3.18`, recorded in Cargo metadata and
  the generated CycloneDX SBOM.

## Verification Baseline

- Locked Rust default matrix: 1,181 passed, zero failed.
- Locked Rust all-feature matrix: 1,181 passed, zero failed.
- Repository Python suite against the installed optimized ABI3 wheel: 260 passed, four skipped.
- Python Stable ABI compiled-document suite: 10 passed.
- Independent reflow verification: 1,000 records, 1,750 pages, 24,900 markers, exact record order,
  and exact local page counters.
- Compact compiled output is byte-identical to ordinary rendering.
- Poppler raster inspection passed for a one-page record and both pages of the final continued
  record, with no clipping, overlap, missing header, or page-counter defect.
- The release workflow builds, installs, and smokes 15 `cp310-abi3` wheels plus one source
  distribution before publication.

The exact `v2.2.4` source is validated before registry publication. GitHub, PyPI, and crates.io are
published from that immutable tagged source, with the complete Python distribution set and its
SHA-256 manifest attached to the GitHub release.
