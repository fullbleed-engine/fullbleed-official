# Fullbleed Release Notes - 2.2.1

Status: release

Release date: August 9, 2026

## Summary

Fullbleed 2.2.1 closes the current IronPress parity pass and extends the compiled variable-data
lane through immutable page-space transforms and clips. The official independent Wheel265 gate
reports 1,642 comparator passes, zero implementation failures, and 20 reference-disputed cases.
The disputed cases remain visible in the evidence set and are not relabeled as engine passes.

The authoritative layout and geometry representation remains signed Q32.32 fixed point. Raster
coverage and PDF serialization use bounded conversion only at their defined output boundaries.

## CSS, Layout, and Reflow

- Hardens fixed-height block, one-column grid, and column-flex continuation slicing without
  duplicating content across fragmentainers.
- Preserves `display: contents` grid descendants and paint phase through continuations.
- Retains empty styled captions in avoided figures and keeps positioned containing blocks atomic.
- Expands generated-content, anonymous-table, positioning, counter, marker, footnote, text,
  writing-mode, bidi, mask/filter, SVG, image, and fragmentation behavior exercised by the
  independent suite.
- Uses `unicode-bidi 0.3.18` for deterministic Unicode bidi classes, paragraph levels, and visual
  run ordering instead of ad-hoc character-range direction checks.
- Keeps coverage claims evidence-scoped: reference-disputed fixtures are reported separately, and
  Fullbleed is not presented as a general browser replacement.

## Compiled Variable Data

- Compiles active page-space transform and rectangular/path clip commands into each dynamic
  binding overlay while continuing to share immutable static page content.
- Checkpoints coordinate state across PDF save/restore commands, so a slot replays only the state
  active at its original paint position.
- Preserves the compile-once contract: transformed and clipped records do not rerun HTML parsing,
  selector matching, layout, pagination, or static paint serialization.
- Keeps the current boundary explicit: slots inside form XObjects and values that change shaping,
  wrapping, dimensions, pagination, or tagged structure still require another compiler layer or
  the ordinary renderer.

On the Windows AMD64 benchmark host, a transformed and clipped one-page template rendered 100,000
distinct records in a five-run median of 208.482 ms, or **479,657 pages/s**. The 47,762,773-byte
output was deterministic with SHA-256
`5d698871971f1df63a19b079e27b85c89569fcc0dbe9c653eacb9447e84fa98d`.

The existing six-field invoice workload rendered 100,000 distinct records in a five-run median of
290.003 ms, or **344,824 pages/s**. Its 88,139,022-byte output was deterministic with SHA-256
`5e7c40cb1315292061ffd009fcc1dd360306bbe759a3d61f23a1b2e06e10dbe3`.

These numbers describe fixed-geometry, paint-only bindings with distinct record data. They are not
the identical-page copy benchmark and do not claim the same speed for arbitrary reflowing HTML.

## Audit Contract and Reproducibility

- Publishes `fullbleed_audit_contract 0.1.3` with canonical LF registry payloads on every host.
- Computes registry hashes and the aggregate contract fingerprint from those canonical bytes,
  eliminating Windows-versus-Unix checkout differences.
- Retains immutable, deterministic registry lookup behavior and adds a regression gate that rejects
  carriage returns in all public embedded payloads.

## Compatibility

- Adds behavior without removing the 2.2.0 Rust, Python, CLI, batch, compiled-copy, or Stable ABI
  surfaces.
- Retains `cp310-abi3` wheels for standard CPython 3.10 through 3.14 across the 15-platform release
  matrix.
- Retains Rust 1.85 as the minimum supported toolchain.
- Keeps exact TrueType font subsetting and shared-resource PDF linking enabled.

## Package Versions

- `fullbleed 2.2.1` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.3` on crates.io.
- `subsetter 0.2.6`, `rustc-hash 2.1.3`, and `unicode-bidi 0.3.18`, recorded in Cargo metadata and
  the generated CycloneDX SBOM.

## Verification Baseline

- Rust library suite: 1,174 passed, zero failed.
- Repository Python suite: 260 passed, zero failed.
- Python Stable ABI compiled-document suite: eight passed.
- Official independent Wheel265 parity gate: 1,642 passed, zero failed, 20 reference-disputed;
  the durable report records invocation `78173eed3cd2bf2c70127eef63092119` as complete.
- External Poppler 300-DPI rasterization matches ordinary-render PNG hashes for transformed and
  overflow-clipped bindings.
- Both 100,000-record benchmark workloads verify deterministic bytes and page ordering.
- The release workflow builds, installs, and smokes 15 `cp310-abi3` wheels plus one source
  distribution before publication.

The exact `v2.2.1` source is validated before registry publication. GitHub, PyPI, and crates.io are
published from that immutable tagged source, with the complete Python distribution set and its
SHA-256 manifest attached to the GitHub release.
