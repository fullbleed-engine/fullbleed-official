# Fullbleed Release Notes - 2.1.0

Status: release

Release date: August 4, 2026

## Summary

Fullbleed 2.1 is the first compiler and resource-virtualization performance release. It fixes the
largest material output-size problem in 2.0 by embedding exact TrueType glyph subsets, introduces
an immutable compile-once rendering API, and gives fixed compiled batches a shared-content PDF
linking path.

The authoritative layout and geometry representation remains signed Q32.32 fixed point. The
release preserves the dependency-free Rust, Python runtime, and Python build graphs introduced in
2.0.

## Font Subsetting and Output Size

- Embeds deterministic six-letter subset names and only the TrueType glyph closure used by a PDF.
- Recursively retains composite glyph components and rebuilds `glyf`, `loca`, the SFNT directory,
  table checksums, and `head.checksumAdjustment`.
- Flate-compresses embedded subset streams and caches raw and compressed programs in bounded
  per-engine and process-wide caches.
- Resolves shaping before subsetting, allowing unused shaping and variable-font tables to be
  omitted while the default TrueType instance remains valid.
- Retains a safe full-font fallback for unsupported CFF/color/bitmap/SVG programs.

Across the five independent benchmark fixtures, generated PDFs are 95.5-97.3% smaller than
released 2.0.0 while decoded subset programs remain parseable and expected text remains
extractable.

## Compiled Rendering

- Adds Rust `FullBleed::compile_document(...)` and immutable `CompiledDocument` rendering APIs.
- Adds Python `PdfEngine.compile_pdf(...)` plus `CompiledDocument.stats()`, `render_pdf()`,
  `render_pdf_to_file()`, and `render_pdf_batch()`.
- Reuses immutable compiled CSS and page-template contexts through a bounded engine cache.
- Releases the Python GIL during compilation and linking; the same immutable compiled document can
  be rendered concurrently from multiple Python threads.
- Virtualizes identical untagged page copies so distinct ordered page dictionaries reference one
  content stream. Tagged profiles deliberately retain page-specific streams and structure-parent
  records.
- Removes redundant compatibility-JIT bounds scans, placement clones, and result sorting from
  production paths.

## Measured Performance

On the independent Windows AMD64 benchmark host:

- ordinary warm rendering is 2.90x faster by geometric mean than released 2.0.0;
- link-only compiled rendering is 10.71x faster than released 2.0.0 and 3.69x faster than warm
  2.1.0;
- a 20-copy virtualized batch reaches 200.7x per-page geometric-mean throughput versus the released
  2.0.0 warm renderer;
- a 1,000-page fixed-copy stress run sustains 304,479-666,622 pages/s while producing 174-270 KB
  PDFs.

The 200.7x result applies to identical compiled copies in one ordered PDF. It is not a claim for
arbitrary unseen HTML or records with varying layout. The current `PlanAndReplay` mode remains a
compatibility planner; packed vector bytecode, typed binding slots, dependency-based partial
reflow, and SIMD/GPU filter acceleration remain subsequent compiler phases.

See `docs/performance-pass-2026-08-04.md` for the measurement ledger and
`docs/performance-architecture.md` for the compiler and shader roadmap.

## Compatibility

- Adds API without removing the 2.0 `PdfEngine`, asset, batch, CLI, and Stable ABI surfaces.
- Preserves `cp310-abi3` wheels for standard CPython 3.10 through 3.14 across the 15-platform
  matrix.
- Preserves Rust 1.85 as the minimum supported toolchain.
- Preserves deterministic bytes for ordinary and compiled single-document renders.
- Preserves PDF profile behavior; tagged output bypasses content-stream sharing when structure
  semantics require page-local records.
- Keeps both shipped Rust crates and the Python distribution MIT licensed.

## Package Versions

- `fullbleed 2.1.0` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.2` on crates.io, unchanged from 2.0.

## Verification Baseline

- Rust library suite: 824 passed, zero failed.
- Repository Python suite: 254 passed and four skipped.
- Independent Python benchmark/coverage suite: 31 applicable tests passed; one 2.0 assertion that
  required complete source-font embedding is intentionally superseded by subset-program validation.
- Python Stable ABI compiled-document tests: seven passed.
- PyMuPDF and ImageMagick visual comparison of simple and full-document outputs: zero differing
  pixels against released 2.0.0.
- Golden PDF hashes and byte counts were regenerated for font-subset output; every checked-in PNG
  page hash remained unchanged.
- A 1,000-page virtualized PDF reports one shared content stream; pages 1, 500, and 1,000 have
  identical raster hashes, and page 1,000 retains expected extractable text.

The exact `v2.1.0` source is validated by the release workflows before registry publication. The
release set contains 15 `cp310-abi3` wheels, one source distribution, Cargo package evidence, and
matching GitHub release assets.
