# Performance pass results: 2026-08-04

This report compares the Fullbleed 2.1.0 release source with the independently measured Fullbleed
2.0.0 baseline on the same Windows AMD64 host. The independent Python harness used 20 latency
samples after five warmups, a 50 ms target sample, fixed seeds, local Times New Roman files, Unicode
shaping/metrics, and one-page versions of the five upstream IronPress fixtures.

The released 2.0.0 artifacts are the frozen baseline; the optimized measurements describe 2.1.0.

## Warm latency and output size

| Fixture | Released 2.0.0 median | Fullbleed 2.1.0 median | Speedup | Released bytes | 2.1.0 bytes | Reduction |
|---|---:|---:|---:|---:|---:|---:|
| Simple HTML | 3.392 ms | 0.373 ms | 9.10x | 1,192,762 | 32,314 | 97.3% |
| Styled HTML | 4.821 ms | 1.541 ms | 3.13x | 1,193,818 | 45,726 | 96.2% |
| Table HTML | 8.228 ms | 3.137 ms | 2.62x | 2,365,851 | 82,020 | 96.5% |
| Markdown | 7.949 ms | 5.379 ms | 1.48x | 1,194,629 | 53,164 | 95.5% |
| Full document | 14.061 ms | 7.545 ms | 1.86x | 3,332,650 | 128,189 | 96.2% |

The geometric-mean warm speedup over the released Fullbleed baseline is **2.90x**. Every 2.1.0
artifact was classified by the independent validator as `subset-programs`; decoded embedded font
programs remained parseable and expected text remained extractable.

The same-machine IronPress comparison is diagnostic because the harness's strict face/link parity
gate excludes four fixtures. The diagnostic geometric mean is 11.13x for production and 11.46x for
the compatibility JIT. The table fixture passes every strict gate at 10.35x production and 10.90x
JIT.

## New-engine and first-process behavior

The process-wide subset cache improves repeated construction of short-lived engines without hiding
that construction from the timed region:

| Fixture | Per-engine cache only | Process-wide cache | Speedup |
|---|---:|---:|---:|
| Simple HTML | 18.585 ms | 16.860 ms | 1.10x |
| Styled HTML | 18.993 ms | 16.062 ms | 1.18x |
| Table HTML | 23.920 ms | 17.632 ms | 1.36x |
| Markdown | 23.263 ms | 19.857 ms | 1.17x |
| Full document | 36.205 ms | 26.167 ms | 1.38x |

Five separate-process samples put the first-ever HTML render at 21.96 ms simple, 23.64 ms styled,
29.09 ms table, and 33.19 ms full-document median. The first-ever Markdown measurement was 93.71 ms
because that lane also imports Python-Markdown inside the timed conversion; it is not a native-engine
startup measurement.

## Throughput

The equal-wrapper lane returns 20 independent PDFs through the same persistent Python thread-pool
contract for both engines:

| Fixture | Sequential | 20 workers |
|---|---:|---:|
| Simple HTML | 2,458 PDFs/s | 10,516 PDFs/s |
| Styled HTML | 598 PDFs/s | 4,254 PDFs/s |
| Table HTML | 319 PDFs/s | 1,926 PDFs/s |
| Markdown | 181 PDFs/s | 516 PDFs/s |
| Full document | 131 PDFs/s | 1,045 PDFs/s |

Against the same IronPress wrapper, the diagnostic geometric mean is 11.46x sequential and 59.53x
threaded. FullBleed releases the Python GIL; the tested IronPress binding does not.

The separate native batch API writes one ordered 20-page PDF. Production results were 5,827 simple,
2,604 styled, 1,189 table, 427 Markdown, and 535 full-document input pages per second. This lane is a
different output contract and is not compared directly with independent-PDF throughput.

## Compiled fixed-document lane

`PdfEngine.compile_pdf` now freezes the post-layout Q32.32 display document and its linker resources.
The ordinary linker can render that immutable object repeatedly without parsing, selector matching,
layout, pagination, or command planning:

| Fixture | Compile once | Link-only median | Versus warm 2.1.0 | Versus released 2.0.0 |
|---|---:|---:|---:|---:|
| Simple HTML | 1.353 ms | 0.083 ms | 4.49x | 40.86x |
| Styled HTML | 2.576 ms | 0.450 ms | 3.43x | 10.72x |
| Table HTML | 3.378 ms | 0.903 ms | 3.47x | 9.11x |
| Markdown | 5.529 ms | 1.277 ms | 4.21x | 6.22x |
| Full document | 7.074 ms | 2.477 ms | 3.05x | 5.68x |

The link-only geometric mean is 3.69x faster than warm 2.1.0 and 10.71x faster than released
2.0.0. Compilation and execution are reported separately; the compile figure includes
Python-Markdown conversion for the Markdown fixture.

The fixed-copy batch API virtualizes each untagged source page into one content stream and gives
every ordered page dictionary a reference to it. A 20-copy batch produced:

| Fixture | Median for 20 pages | Pages/s | Versus released per-page rate | PDF bytes |
|---|---:|---:|---:|---:|
| Simple HTML | 0.152 ms | 132,013 | 447.8x | 34,938 |
| Styled HTML | 0.530 ms | 37,750 | 182.0x | 48,350 |
| Table HTML | 0.623 ms | 32,129 | 264.3x | 84,644 |
| Markdown | 1.083 ms | 18,463 | 146.8x | 55,788 |
| Full document | 2.728 ms | 7,333 | 103.1x | 130,813 |

This compiled fixed-copy lane reaches a **200.7x geometric-mean per-page improvement** over the
released 2.0.0 warm renderer. It is the first executable virtualization path, but it is deliberately
not presented as dynamic-template performance: all copies have identical content and are linked
into one PDF.

A 1,000-page stress run sustained 304,479-666,622 pages/s and produced 174-270 KB PDFs. The PDF page
tree reported 1,000 pages with one content stream. PyMuPDF raster hashes for pages 1, 500, and 1,000
were identical, and page 1,000 retained all expected extractable text. Tagged profiles bypass stream
sharing and retain page-specific content streams/structure parents.

## Fullbleed 2.2.0 compiled variable-data lane

Fullbleed 2.2.0 adds columnar `{{slot}}` text bindings to `CompiledDocument`. It compiles
HTML/CSS and fixed-point geometry once, lowers all static page paint to one shared stream, and emits
one compact record-specific overlay stream per page. Unlike `render_pdf_batch`, these pages are not
immutable copies: the benchmark binds six independently supplied invoice columns and gives every
record a unique invoice ID.

The optimized `cp310-abi3` wheel was measured on the same Windows AMD64 host with Python 3.11.8.
Each figure is the median of five 100,000-record runs and includes Python-to-Rust column conversion:

| Output lane | Median | Pages/s | Including one-time 4.168 ms compile |
|---|---:|---:|---:|
| In-memory PDF | 283.807 ms | 352,352 | 347,252 pages/s |
| Direct flushed file | 295.999 ms | 337,839 | 333,147 pages/s |

The PDF is 88,138,877 bytes. The harness verified exactly 100,000 page dictionaries and all invoice
IDs from `INV-000000000` through `INV-000099999` in page order, one shared static content stream,
100,000 unique dynamic content streams, and no unresolved markers. It obtained identical SHA-256
`2fdb268240bf52f87cd75670dac815ce8a6e29603ecfbdc344280df06c0b2540` from every buffer and direct-file
run. PyMuPDF independently parsed the 100,000-page result; pages 1, 50,000, and 100,000 extracted
their expected distinct values and rasterized at 612 x 792 pt without clipping.

This path does not use multiprocessing. Its throughput comes from removing per-record HTML parsing,
style resolution, layout, pagination, and static paint serialization. A reusable binding buffer and
buffered file writer keep allocation and syscall counts bounded while the linker preserves page
order. The Python GIL is released during native execution.

## Fullbleed 2.2.4: compiled content reflow

Fullbleed 2.2.4 adds
`CompiledDocument.render_pdf_reflow_bindings` and its direct-file counterpart. This is a separate
program from the fixed-geometry overlay linker. Initial compilation parses and recovers the static
template DOM, lowers `{{slot}}` text nodes into immutable binding programs, and retains the CSS
resolver, page templates, fonts, and engine configuration. Ordinary slots are literal text;
explicit empty `data-fb-bind-html="slot"` targets accept trusted generated structure such as
narrative blocks and table rows.

Flow is now a real compiler target. On the first encounter with a structural/input shape, a worker
runs the ordinary signed-Q32.32 layout, fragmentation, pagination, and paint planner and captures a
guarded `CompiledFlowRecordProgram`. Later records try those programs before materializing a DOM.
Every dynamic paint run retains its fixed-point origin, alignment box, parent fit guard, spacing,
and browser text-paint phase. A record that fits binds directly; a guard miss executes layout once
and adds a new variant. This preserves content-driven page counts while moving the hot path from
full layout to bind/shape/execute/link.

The PDF stage compiles each eligible page into static vector segments plus text paint slots.
Workers lower bound slots to pre-shaped TJ operators, collect only newly encountered glyphs,
instantiate the page program, and Deflate the content before sending ordered record results to the
linker. The implementation uses native Rust scoped threads, not Python multiprocessing. On this
20-logical-processor host it selected 20 workers, an 80-record bounded window, and 512-page linker
flushes. The Python GIL is released during native execution.

The final `cp310-abi3` wheel was independently measured on a Windows AMD64 Intel Core
i7-12700H/Python 3.11 host. The adapter used nine literal slots and three trusted structural slots.
The first sample compiled flow variants on demand; the next 29 rendered the same 1,000 distinct
binding rows through the hot programs and wrote a new PDF each time:

| Measurement | Ordinary render | Compiled first discovery | Compiled hot median |
|---|---:|---:|---:|
| Direct-file render | 9.470 s | 0.509 s | **216.160 ms** |
| Records/s | 105.6 | 1,963.9 | **4,626.2** |
| Pages/s | 184.8 | 3,436.8 | **8,095.8** |
| Output bytes | 5,298,961 | 6,870,320 | 6,870,320 |

The 29 hot samples ranged from 168.995 to 269.975 ms. The best sample reached 5,917.3 records/s
and 10,355.3 pages/s; the earlier 168.839 ms release-note number was therefore reproduced as a
best-case result, not as the independent median. The defensible hot-median speedup is 43.8x over
ordinary rendering. A complete cold job including engine/font setup, binding-column construction,
document compilation, first variant discovery, and output took 0.588 s: about 1,700 records/s,
2,975 pages/s, and 16.2x the ordinary cold path. Novel structure or values that violate every
existing geometry guard still pay a new variant compilation before joining the hot lane.

The workload contract remains exact: 1,000 variable records, 1,750 naturally generated pages, the
500/300/150/50 distribution of 1/2/3/4-page records, all 24,900 markers once and in order, and all
1,750 local page counters. The throughput default uses a four-step deterministic Deflate search;
all 29 hot repetitions produced 6,870,320 bytes with SHA-256
`1dd02748af497ae2f477875ecf0ab87a6de5ceb55e097b4b775ddbf6d5038194`. The independently measured
compact hot median was 0.260 s, 3,850.5 records/s, and 6,738.4 pages/s. In current source,
`CompiledFlowCompression.Compact` selects that deterministic 64-step search per render and can be
mixed safely with throughput calls in one process. Compact mode reproduced the ordinary PDF byte
for byte: 5,298,961 bytes and SHA-256
`bb3c441313a08fb00d3bd15f23a567981f3bbbd550908e3a9fe7a07ca5d7f138`. The independent checker
verified all markers for that parity artifact as well.

## Validation

- Independent 2.2.4 case study: 54 tests passed, `pip check` reported no broken requirements, all
  24,900 markers were verified, extracted text matched on all 1,750 pages, and sampled raster
  hashes matched across ordinary, throughput, and compact output.
- `cargo test --lib`: 824 passed, zero failed.
- Repository Python suite: 254 passed and four skipped.
- Independent Python suite: 31 passed. One old assertion was deselected because it requires
  `complete-source-programs`, the exact behavior this pass intentionally replaces.
- PyMuPDF rendered the simple and full-document PDFs at 2x; ImageMagick absolute-error comparison
  against the released renderer was zero pixels for both.
- Production and compatibility-JIT PDFs are byte-identical for the five benchmark fixtures.
- The authoritative geometry and layout representation remains signed Q32.32 fixed point.
- Fullbleed 2.2.0 source: `cargo test --lib` reports 901 passed, zero failed; the repository
  Python suite reports 256 passed and four skipped against the optimized wheel.

## Scope of the result

Fullbleed 2.1.0 implements TrueType glyph closure, deterministic subset naming, Flate-compressed embedding,
bounded raw/compressed subset caches, immutable CSS/page-template reuse, linker counters, removal of
redundant compatibility-JIT scans/clones, an immutable compiled-document API, and shared-content page
virtualization. The current `PlanAndReplay` JIT remains a compatibility planner, and the compiled
document still holds the existing command enum rather than packed vector bytecode. Fullbleed 2.2.0
adds fixed-geometry text slots. Fullbleed 2.2.4 adds parsed-DOM full-record
reflow plus guarded structural flow programs and compiled PDF page-paint shaders. Current source
also makes compiled-flow compression a per-call enum, accepts `content(text)` named-string capture,
and relaxes oversized keeps before deferring splittable content. General typed
size policies, dependency-based partial reflow, and row virtualization remain future phases. The
200.7x result applies only to fixed compiled copies; the 337,839 pages/s direct-file result applies
to distinct paint-only text bindings; and the independently measured 8,095.8 pages/s hot median
(10,355.3 pages/s best) applies to compiled variants of the exact variable-length reflow case study
above. None applies to arbitrary
previously unseen HTML without the corresponding compile contract.
