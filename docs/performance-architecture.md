# Compiled rendering architecture

This document defines the performance contract that follows the independent FullBleed 2.0.0
benchmark. It separates optimizations for arbitrary HTML from the much faster contract available
when a document family is compiled once and rendered many times.

The measured phase-one implementation ships in Fullbleed 2.1.0 and is recorded in
`performance-pass-2026-08-04.md`.

## Workload lanes

1. **Cold arbitrary HTML** parses, styles, lays out, shapes, and links a previously unseen input.
   Correctness and bounded resource use take priority; a 200x claim does not apply to this lane.
2. **Warm HTML with stable CSS** reuses the immutable stylesheet, selector indexes, page templates,
   font programs, and resource metadata. HTML and layout remain dynamic.
3. **Compiled template** fixes DOM structure and geometry and exposes value slots. Paint-only text
   bindings execute today; dependency-based invalidation for size-changing values remains the next
   compiler layer. This is the primary 50-200x target lane.
4. **Compiled batch** executes columnar fixed-geometry bindings, emits ordered page fragments, and
   links shared resources once. Parallel fragment production remains future work; the current
   ordered single-process path already exceeds the 100,000 pages/s gate.

The implementation exposes `compile_pdf`, a virtualized fixed-copy batch, and a distinct-record
fixed-geometry binding batch. It freezes the existing fixed-point command display list, shares
static page content/resources, and lowers `{{slot}}` text runs into compact per-record overlay
streams. Typed size policies, partial reflow, complex-script reshaping, and packed bytecode remain
later phases.

Every benchmark must name its lane. Repeated-input memoization is not a compiled-template result.

## Pipeline

```text
HTML/CSS template
      |
      v
frontend compiler ----> immutable style/selector program
      |
      v
layout dependency graph <---- typed binding slots
      |
      v
fixed-point vector program ----> PDF vector lowerer ----> ordered resource linker
      |                                  |
      +----> deterministic CPU raster ---+
      |
      +----> optional SIMD/GPU shader backend for filters, masks, images, and previews
```

### Frontend compiler

- Parse CSS, `@page`, selectors, and the static DOM once.
- Intern strings, selectors, computed-style deltas, assets, and font identities.
- Store positional/sibling selector dependencies explicitly so a binding cannot silently reuse an
  invalid style result.
- Produce a stable template fingerprint from source, engine options, assets, fonts, and page setup.

### Virtualized layout

- Compile a dependency DAG for intrinsic sizes, line boxes, tracks, fragmentation, and page breaks.
- Give each binding slot a type, maximum encoded size, affected-node set, and reflow policy.
- Patch paint-only values directly. Recompute the smallest valid subgraph for size-affecting values.
- Virtualize repeated rows/items so only the visible/current page window is materialized during
  pagination; retain compact page-break checkpoints instead of the complete box tree.
- Fall back to the ordinary layout engine whenever a dependency cannot be proven safe.

### Vector program

The vector program is immutable packed bytecode, not a cloned `Vec<Command>` replay. It contains:

- state operations: save/restore, transforms, clips, opacity, blend modes;
- geometry streams: rectangles, paths, glyph positions, image/form quads;
- paint records: solid colors, gradients, shadings, masks, and filter graphs;
- resource handles: fonts, glyph closures, images, forms, and optional-content groups;
- typed slot references and page-fragment boundaries.

Coordinates and layout values remain signed Q32.32. PDF lowering uses deterministic fixed-point
formatting. A GPU backend may use floating-point internally only behind a quantized boundary and
must match the CPU reference within the visual regression tolerance.

### PDF lowering and resource virtualization

- Pre-lower static bytecode ranges into reusable PDF content fragments.
- Shape static text once and cache dynamic runs by font, features, language, direction, and text.
- Accumulate exact glyph closure while compiling, then reuse cached raw and compressed subsets.
- Allocate virtual resource handles during parallel work; assign deterministic PDF object numbers
  in one ordered linker pass.
- Deduplicate fonts, images, forms, shadings, and graphics states across the complete batch.
- Stream pages in order with bounded queues so document count does not determine peak memory.

PDF content is already vector data. GPU shaders are therefore aimed at filters, raster fallbacks,
image transforms, masks, and previews; they do not replace the PDF vector lowerer or justify a
throughput claim by themselves.

## JIT contract

The existing `PlanAndReplay` mode is a compatibility planner: it clones display commands and
replays them. It is not the final JIT. The compiled implementation must have explicit phases:

1. `compile_template`: frontend, dependency graph, vector bytecode, and virtual resources;
2. `bind`: validate and encode typed slot values;
3. `execute`: invalidate/reflow bounded nodes and generate page fragments;
4. `link`: resolve resources, subset fonts, and serialize deterministic PDF bytes.

Compilation time, first render, warm render, batch throughput, output bytes, and peak memory are
reported separately. A compile cache hit must be observable in performance counters.

## Performance and correctness gates

- Compare against the frozen independent 2.0.0 results on the same machine and harness.
- Report medians and p95 for cold, warm, compiled-template, and batch lanes separately.
- Require deterministic bytes for identical inputs and thread counts unless a profile explicitly
  permits nondeterministic metadata.
- Require valid embedded subset names, parseable decoded font programs, exact composite closure,
  text extraction, glyph audit, and PDF profile validation.
- Render representative PDFs with Poppler and compare page geometry and pixels before release.
- Reject improvements that move unbounded work or memory into preflight without reporting it.
- Treat 200x as a stretch target for fixed-geometry compiled templates and large batches, not as a
  claim for previously unseen arbitrary HTML/CSS.

## Delivery sequence

1. **Linker and warm frontend:** exact font subsetting/compression caches, stylesheet/page-template
   cache, linker counters, removal of redundant replay work, immutable fixed-document compilation,
   and shared-content fixed-copy virtualization.
2. **Packed vector IR:** fixed-point bytecode, interned resources, direct PDF lowering, and no
   intermediate `Document` reconstruction.
3. **Typed template bindings:** dependency graph, paint-only patching, bounded partial reflow, and
   public Rust/Python compile-bind APIs. The public columnar API and paint-only fixed-geometry
   patching are implemented; the dependency graph and bounded reflow are not.
4. **Parallel virtual linker:** virtual object handles, persistent worker execution, ordered bounded
   streaming, and batch-wide resource closure.
5. **Shader backend:** SIMD first, optional GPU filters/rasterization second, always checked against
   the deterministic CPU reference.
