# Static HTML/CSS renderer comparison

An optional, maintainer-run comparison of Fullbleed 2.5.6, WeasyPrint 70.0,
and Playwright 1.63.0 / its bundled Chromium. This is not a browser-parity,
standards-conformance, accessibility, or general performance ranking.
The dependencies here are separate from the Fullbleed package.

## Contract declared before measurement

All three adapters receive the same generated HTML and CSS: a one-page invoice,
a 100-row ledger (three or four pages permitted), and a report with three explicit
pages. A4 paper, 36-point margins, the bundled OFL Inter static font at weight 400,
print backgrounds, and synthetic records are shared. No network assets or
JavaScript content are involved. Font bytes are supplied through the shared
CSS data URL; Fullbleed also registers that same font explicitly through its API.
The CSS, font hashes, HTML, acceptance contracts, and adapter source are retained.
The static font is prepared once with `prepare_font.py` and pinned fontTools,
with optical size 14 and weight 400. Font preparation is outside every timing.

During initial qualification, Chromium's output with Inter's contextual
alternates enabled extracted uppercase-ID hyphens as `U+E088` in pypdf.
The static font reproduced this too; the variable font additionally produced
Type 3 glyph fonts. These qualification runs are retained separately and are
not latency comparisons. A focused probe confirmed that disabling `calt`
preserved the hyphens. All measured fixtures use the same static instance and
`font-feature-settings: 'calt' 0` in all three engines. This finding is specific
to these versions and font; it does not establish a general browser font limit.

Before a timing can qualify, independent PDF readers must find the declared page
count and A4 dimensions, every record exactly once and in order, the expected
row values and totals, embedded Inter fonts, and visible character boxes inside
the page's 30-point inset. Ledger pages containing rows must repeat the table
heading. Report section markers must start on the declared pages. These checks
do not establish perfect visual equivalence: review every representative page
image too. Differences and failures belong in the results, even when unfavorable.
Negative controls deliberately change the expected record, remove a page,
remove font embedding, and move text beyond the declared inset. Every control
must fail qualification for each renderer.

Visual qualification found that Chromium places the ledger's closing note on
a fourth page; Fullbleed and WeasyPrint fit it on the third. The original ledger
contract allows three or four pages. All 100 records and the total remain intact.
Timings are per complete document, and page counts are reported alongside them.

Timing uses ordinary document APIs, with a different fixed-length `RUN-000001`
token for every render. The output is returned to Python and written to a local
file by the same code in every adapter. Filesystem writes are buffered; there is
no `fsync`. Input generation and validation are outside timing.

- **Fresh process:** five samples per renderer/fixture. The controller measures
  process start through exit, including imports, input reads, font setup, browser
  launch where applicable, rendering, output writing, and cleanup. This is not
  a cold filesystem cache or a reboot benchmark.
- **Warm process:** thirty samples per renderer/fixture, split across three
  independently started blocks of ten, with two untimed warm-ups per block.
  Reuse the engine, parsed WeasyPrint stylesheet/font configuration, or Chromium
  browser/page. Timing includes changed HTML parsing, layout, PDF encoding,
  and the common file write. Chromium waits for `document.fonts.ready`.
- **Memory:** a separate pass samples summed process-tree RSS every 10 ms from
  startup through two warm-ups and three renders. It includes Chromium/driver
  descendants. RSS can double-count shared pages and miss short peaks. It is
  not a per-document allocation figure. Memory sampling does not run during the
  latency measurements.

Renderer/fixture order is shuffled using seed 20261004 for each cold round and
warm block. Work runs sequentially, with no competing benchmark workers. All
samples, including outliers, are retained. Summary statistics are median,
minimum, maximum, and inclusive quartiles; no latency threshold is enforced
on shared CI. No HTTP request, queue, concurrency, compiled-template, VDP, or
browser-content workload is measured.

## Reproduce

Use Python 3.10 or newer on Linux. WeasyPrint requires Pango and related shared
libraries, and Chromium requires its browser libraries. Follow the projects'
installation documentation if your environment lacks them:

- <https://doc.courtbouillon.org/weasyprint/stable/first_steps.html>
- <https://playwright.dev/python/docs/browsers>

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r benchmarks/pdf_renderers/requirements.txt
.venv/bin/python -m playwright install chromium
.venv/bin/python benchmarks/pdf_renderers/compare.py --out target/pdf-comparison
```

Run on a local Linux filesystem, not a network share or a Windows-mounted WSL
path. The measured output directory must be new. Use `--smoke` for one cold
sample and two warm samples per pair, plus the same qualification checks.
Smoke runs validate the harness; they are not publishable performance results.
The controller gives every worker the virtual environment's executable directory
plus the standard Linux PATH. An initial WSL diagnostic found that 82 inherited
Windows PATH entries added roughly ten seconds to WeasyPrint's shared-library
discovery at import. Those host-specific paths are excluded for all engines;
the initial import profiles are retained with the qualification notes.

The published run records its exact transitive packages, browser, operating
system, CPU, source revision, and raw results. Results from one WSL host cannot
predict every deployment. Render your own documents and inspect the PDFs before
using timings to choose a library.
