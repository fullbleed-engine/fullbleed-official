# Compiled rendering architecture

This document defines the performance contract that follows the independent FullBleed 2.0.0
benchmark. It separates optimizations for arbitrary HTML from the much faster contract available
when a document family is compiled once and rendered many times.

The measured phase-one implementation ships in Fullbleed 2.1.0 and is recorded in
`performance-pass-2026-08-04.md`. Fullbleed 2.2.4 also contains the guarded flow compiler and PDF
page-paint shader; its measured boundary is recorded in that document.

## Workload lanes

1. **Cold arbitrary HTML** parses, styles, lays out, shapes, and links a previously unseen input.
   Correctness and bounded resource use take priority; a 200x claim does not apply to this lane.
2. **Warm HTML with stable CSS** reuses the immutable stylesheet, selector indexes, page templates,
   font programs, and resource metadata. HTML and layout remain dynamic.
3. **Compiled template** fixes template structure and exposes two execution policies. Paint-only
   text slots reuse frozen geometry and immutable page-space transform/clip state; this is the
   primary 50-200x lane. Reflow slots compile encountered structural/input variants into guarded
   fixed-point display programs. A matching record binds, shapes, and executes the program without
   layout; a guard miss runs full layout once and adds a variant. Explicit trusted structural slots
   can vary paragraphs, table rows, and similar child content. Their values are parsed as markup,
   so applications must sanitize untrusted HTML or keep user data in literal scalar slots.
4. **Compiled batch** accepts exact columnar bindings and links ordered output once. Fixed-geometry
   overlays use the virtual linker and exceed the 100,000 pages/s gate. Reflow workers bind guarded
   programs, lower cached PDF page segments and text slots, precompress page streams, and feed an
   ordered bounded linker. Batch-wide resource closure and virtualized variable rows remain future
   work.

The implementation exposes `compile_pdf`, a virtualized fixed-copy batch, a distinct-record
fixed-geometry binding batch, and a distinct-record compiled reflow batch. The fixed lane freezes
the existing fixed-point command display list, shares static page content/resources, and lowers
`{{slot}}` text runs into compact per-record overlay streams. The reflow lane lowers the recovered
DOM into an immutable node/text-binding blueprint, updates worker-local text cells without
reparsing the template, caches trusted structural inputs, and compiles guarded flow variants. Hot
records instantiate fixed-point text constraints directly. Eligible PDF pages are separately
compiled into static vector segments and text-paint slots. General typed size policies,
dependency-bounded partial reflow, row virtualization, batch-wide resource closure, and a packed
layout IR remain later phases.

Compiled reflow compression is an explicit per-job policy. Throughput mode uses a bounded search
for large page streams; compact mode uses the deterministic full search. The option is threaded
through worker encoding and linker fallback, so calls with different policies can run sequentially
or concurrently without process-global state.

Every benchmark must name its lane. Repeated-input memoization is not a compiled-template result.

## Pipeline

```text
HTML/CSS template
      |
      v
frontend compiler ----> immutable style/selector program
      |
      v
structural flow key <---- literal/structural binding columns
      |
      +---- cache miss ----> fixed-point layout/pagination ----> guarded flow program
      |                                                        |
      +---- cache hit -----------------------------------------+
                                                               v
bound fixed-point paint slots ----> PDF page shader ----> worker Deflate ----> ordered linker
```

### Frontend compiler

- Parse CSS, `@page`, selectors, and the static DOM once.
- Intern strings, selectors, computed-style deltas, assets, and font identities.
- Store positional/sibling selector dependencies explicitly so a binding cannot silently reuse an
  invalid style result.
- Produce a stable template fingerprint from source, engine options, assets, fonts, and page setup.

### Virtualized layout

- Implemented: cache structural/input variants and capture their authoritative fixed-point display
  programs, text widths, alignment boxes, parent fit guards, spacing, and browser paint phases.
- Implemented: bind a record only when every dynamic value satisfies the captured geometry;
  otherwise run ordinary layout once and add a variant.
- Next: compile a dependency DAG for intrinsic sizes, line boxes, tracks, fragmentation, and page
  breaks so a miss can recompute the smallest valid subgraph.
- Next: virtualize repeated rows/items with compact page-break checkpoints rather than a complete
  box tree.

### Vector program

The implemented PDF-target program splits an eligible page into immutable static PDF vector
segments and typed text-paint slots. A worker writes only slot x coordinates and pre-shaped TJ/text
operators between those segments, then compresses the completed page stream. Unsupported resource
or transform cases retain a deterministic ordinary-lowering fallback. A fully packed, target-neutral
layout/vector bytecode replacing every `Vec<Command>` is still future work.

Coordinates and layout values remain signed Q32.32. PDF lowering uses deterministic fixed-point
formatting. A GPU backend may use floating-point internally only behind a quantized boundary and
must match the CPU reference within the visual regression tolerance.

### PDF lowering and resource virtualization

- Pre-lower static bytecode ranges into reusable PDF content fragments.
- Shape static text once and cache dynamic runs by font, features, language, direction, and text.
- Use a compiled numeric glyph shader for identifier/date/amount variants, with native shaping as
  the exact fallback.
- Accumulate worker-local glyph discoveries, merge exact closure in record order, and reuse cached
  raw and compressed subsets.
- Allocate virtual resource handles during parallel work; assign deterministic PDF object numbers
  in one ordered linker pass.
- Deduplicate fonts, images, forms, shadings, and graphics states across the complete batch.
- Stream pages in order with bounded queues so document count does not determine peak memory.

PDF content is already vector data. The current “shader” is therefore a deterministic CPU program
that writes PDF text/vector operators; GPU shaders remain aimed at filters, raster fallbacks, image
transforms, masks, and previews.

## JIT contract

The existing `PlanAndReplay` mode remains a compatibility planner. The compiled flow lane now has
explicit phases:

1. `compile_template`: frontend, DOM binding blueprint, CSS/page/font state, and fixed base paint;
2. `bind`: select a structural flow key and validate values against guarded fixed-point programs;
3. `execute`: instantiate the program, shape dynamic runs, and lower/Deflate page paint bytecode;
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
   intermediate `Document` reconstruction. PDF page segment/slot programs are implemented; the
   general target-neutral replacement for the display-command document is not.
3. **Typed template bindings:** dependency graph, paint-only patching, bounded partial reflow, and
   public Rust/Python compile-bind APIs. The public columnar API, paint-only fixed-geometry patching,
   parsed-DOM text/structural binding programs, guarded full-flow variants, fixed-point text paint
   constraints, and on-demand miss compilation are implemented. Slot size policies and
   smallest-valid-subgraph invalidation are not.
4. **Parallel virtual linker:** virtual object handles, persistent worker execution, ordered bounded
   streaming, and batch-wide resource closure. Native scoped workers, bounded ordered scheduling,
   worker-side page compression, and replay-safe page-program caches are implemented; general
   batch-wide virtual resources are not.
5. **Shader backend:** the deterministic PDF page-paint and numeric-glyph CPU shaders are
   implemented. Broader SIMD kernels and optional GPU filters/rasterization remain later work and
   must be checked against the deterministic CPU reference.
