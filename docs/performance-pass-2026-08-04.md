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

## Validation

- `cargo test --lib`: 824 passed, zero failed.
- Repository Python suite: 254 passed and four skipped.
- Independent Python suite: 31 passed. One old assertion was deselected because it requires
  `complete-source-programs`, the exact behavior this pass intentionally replaces.
- PyMuPDF rendered the simple and full-document PDFs at 2x; ImageMagick absolute-error comparison
  against the released renderer was zero pixels for both.
- Production and compatibility-JIT PDFs are byte-identical for the five benchmark fixtures.
- The authoritative geometry and layout representation remains signed Q32.32 fixed point.

## Scope of the result

Fullbleed 2.1.0 implements TrueType glyph closure, deterministic subset naming, Flate-compressed embedding,
bounded raw/compressed subset caches, immutable CSS/page-template reuse, linker counters, removal of
redundant compatibility-JIT scans/clones, an immutable compiled-document API, and shared-content page
virtualization. The current `PlanAndReplay` JIT remains a compatibility planner, and the compiled
document still holds the existing command enum rather than packed vector bytecode. Dynamic typed
slots, dependency-based partial reflow, and shader acceleration remain future phases. The 200.7x
result applies only to fixed compiled copies in one PDF, not arbitrary unseen HTML or varying records.
