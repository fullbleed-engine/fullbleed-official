# Fullbleed Release Notes - 2.2.0

Status: release

Release date: August 5, 2026

## Summary

Fullbleed 2.2 turns the compile-once path into a genuine variable-data engine. A compiled document
can bind distinct columnar records directly into fixed page geometry without repeating HTML parsing,
selector matching, layout, pagination, or static paint serialization. This release also carries the
current CSS/layout coverage expansion, native vector/raster improvements, and exact TrueType font
subsetting that keeps generated PDFs materially smaller than 2.0.

The authoritative layout and geometry representation remains signed Q32.32 fixed point. The fast
path changes how already-resolved work is linked; it does not replace fixed-point layout with
floating-point geometry.

## Compiled Variable Data

- Adds Rust `CompiledDocument::render_bindings_to_buffer`, `render_bindings_to_writer`, and
  `render_bindings_to_file` APIs.
- Adds Python `CompiledDocument.render_pdf_bindings(...)` and
  `render_pdf_bindings_to_file(...)` APIs through the CPython Stable ABI.
- Compiles `{{slot_name}}` markers into page-local text slots and accepts equal-length columnar
  values, producing one ordered page set per record.
- Shares one immutable static content stream while emitting a compact, distinct dynamic text stream
  for every record page.
- Reuses native binding buffers and uses a buffered direct-file writer to bound allocation and
  syscall overhead.
- Releases the Python GIL for native rendering. This lane does not use Python multiprocessing;
  throughput comes from eliminating repeated frontend, layout, and static serialization work.

The existing `render_pdf_batch(...)` path remains the right operation for identical compiled copies.
The new binding methods are the distinct-record path: the output contains different text content for
every record rather than 100,000 references to one immutable page.

## Measured Performance

On the Windows AMD64 benchmark host, a compiled one-page invoice with six bound fields rendered
100,000 distinct records with these five-run medians:

- in-memory: 283.807 ms, or **352,352 pages/s**;
- direct flushed file: 295.999 ms, or **337,839 pages/s**;
- direct file including the one-time 4.168 ms compilation: **333,147 pages/s**.

The 88,138,877-byte PDF was verified to contain exactly 100,000 ordered page dictionaries, all
invoice IDs from `INV-000000000` through `INV-000099999`, one shared static stream, 100,000 unique
dynamic streams, and no unresolved markers. Buffer and file output matched SHA-256
`2fdb268240bf52f87cd75670dac815ce8a6e29603ecfbdc344280df06c0b2540`. PyMuPDF independently
extracted and rasterized records 1, 50,000, and 100,000.

The benchmark is reproducible with:

```bash
python tools/benchmark_fullbleed_vdp.py --records 100000 --repeats 5
```

These figures apply to fixed-geometry, paint-only text bindings. Values that change wrapping,
element dimensions, pagination, complex-script shaping, or accessibility structure still require
the ordinary renderer. The independent fixed-copy lane remains separately measured at more than
200,000 pages/s at 100,000-page scale.

## Fonts, CSS, and Native Rendering

- Retains deterministic TrueType glyph closure and exact subset embedding, including composite
  glyph components, rebuilt SFNT tables, compressed streams, and safe fallback for unsupported
  font formats.
- Uses the audited `subsetter 0.2.6` implementation for broader subset-program correctness while
  preserving deterministic subset naming and caching.
- Expands CSS parsing, cascade, layout, and paint coverage across backgrounds and blend modes,
  borders and outlines, lists and counters, table conflict resolution, logical properties and
  vertical writing modes, filters, clipping, generated content, and native image/SVG paths.
- Keeps the coverage claim evidence-scoped: Fullbleed is not presented as a complete browser CSS
  implementation, and unsupported or partial behavior remains documented in `docs/css-coverage.md`.

## Compatibility and Boundaries

- Adds APIs without removing the 2.1 `PdfEngine`, asset, CLI, batch, compiled-copy, or Stable ABI
  surfaces.
- Preserves `cp310-abi3` wheels for standard CPython 3.10 through 3.14 across the 15-platform
  release matrix.
- Preserves Rust 1.85 as the minimum supported toolchain.
- Keeps Q32.32 fixed-point geometry authoritative in ordinary and compiled lanes.
- Requires exact slot keys, non-empty equal-length columns, page-local WinAnsi text, and untagged
  output for the current variable-data fast path.
- Keeps the compatibility `PlanAndReplay` JIT as a planner. Packed vector bytecode, typed sizing
  policies, dependency-based partial reflow, and SIMD/GPU shader acceleration remain future work.

## Package Versions

- `fullbleed 2.2.0` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.2` on crates.io, unchanged.
- `subsetter 0.2.6` and transitive `rustc-hash 2.1.3`, both recorded in Cargo metadata, license
  evidence, and the generated SBOM.

## Verification Baseline

- Rust library suite: 901 passed, zero failed.
- Repository Python suite: 256 passed and four skipped.
- Python Stable ABI compiled-document suite: eight passed.
- Independent CSS/coverage review suite: 33 passed.
- The 100,000-record benchmark verifies deterministic bytes, page order, unique values, stream
  topology, extraction, and representative rasterization.
- The release workflow builds, installs, and smokes 15 `cp310-abi3` wheels plus one source
  distribution before publication.

The exact `v2.2.0` source is validated before registry publication. GitHub, PyPI, and crates.io are
published from that immutable tagged source, with the complete Python distribution set and its
SHA-256 manifest attached to the GitHub release.
