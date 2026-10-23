# Reproduce PDF link-reader observations

This optional developer check separates four observations: analytic CSS shapes,
the final PDF's `QuadPoints`, independently rendered paint, and reader hit tests.
It is not part of `pip install fullbleed` and adds no runtime dependency.

Use a candidate wheel that includes [PR #125](https://github.com/fullbleed-engine/fullbleed-official/pull/125).
Published `2.6.0a13` does not contain this navigation implementation. The input
manifest records the installed native module's SHA-256 because the local QA
wheel reused that version number.

## Render with the candidate

Run from the repository root in the Python environment containing that wheel:

```sh
python tools/pdf_link_readers/render_cases.py --out target/pdf-link-inputs
```

This writes four HTML/CSS inputs, PDFs, native previews, and `inputs.json`. The
output directory must not already exist. No network access is needed to render.

## Observe independent readers

The reader environment can be separate from the engine environment. The
recorded run used CPython 3.11 and the package versions pinned here:

```sh
python -m pip install -r tools/pdf_link_readers/requirements.txt
npm install --ignore-scripts --no-save --prefix target/pdf-link-reader-deps pdfjs-dist@6.4.299
python -m playwright install --no-remove chromium
python tools/pdf_link_readers/check.py --inputs target/pdf-link-inputs --pdfjs target/pdf-link-reader-deps/node_modules/pdfjs-dist --out target/pdf-link-reader-results
```

Use `--chromium /path/to/chrome` to select an existing Chromium executable.
[Playwright 1.63](https://playwright.dev/python/docs/release-notes#version-163)
supplies Chromium 153.0.8010.12, the browser used for the recorded observations.
Older Chromium 133 cannot load these PDF.js specimens. A different browser
version is recorded as a new observation. Every PDF.js asset used by the harness is checked
against `pdfjs-lock.json`, which also identifies the upstream archive and source
revision. Install the dependencies before running; the check itself only serves
local fixture files on a temporary loopback port and blocks external requests.
Clicks are intercepted before they can follow the synthetic `example.invalid`
links. It opens a fresh headless browser and closes it after the run.

Exit codes:

- **0:** every sampled geometry, paint, and reader observation agrees.
- **2:** observations completed, with mismatches retained in `report.json`.
- **1:** invalid inputs, missing annotations, or an incomplete reader run.

The measured candidate returns **2** because the sloped-link reader differences
are still present. There is no expected-failure list or option to turn those
differences into a passing result. Results are reported independently for each
reader and each shape. Source PDFs, screenshots, individual point observations,
actual click checks, input hashes, runtime identity, reader versions, and the
verification harness are retained in the new output directory.

## Interpret the result

The four specimens occupy a 200pt square page. The base link is a 100pt square
at CSS `(40pt, 40pt)` with a blue background. Variants rotate a 100pt by 40pt
rectangle by 30 degrees, clip a right triangle, or clip a 40pt-radius circle.
The independent oracle derives these shapes analytically from their dimensions.
It does not use Fullbleed's region code or treat reader output as ground truth.

The grid samples every 5pt, excluding points within 1.5pt of the intended
boundary. Paint is sampled from a PDFium render at 144dpi; these specimens have
no decoration outside the link. PDFium observations use `FPDFLink_GetLinkAtPoint`.
PDF.js observations use its parser, canvas, official annotation layer, and
browser hit testing. Selected interior, exterior, and mismatching points also
receive actual mouse clicks. No custom shape filter is added to either reader.

The [document guide](../../docs/pdf-links.md#reader-behavior) records the initial
results and their limits. The compact [recorded observation](observations/e7ae956.json)
includes runtime and reader identity, the per-case mismatch counts and click
observations, and hashes of the complete retained artifacts. A complete run can expose reader differences even when
the encoded geometry and painted shape agree. Treat those as unresolved
compatibility observations, not as a passed conformance or accessibility gate.
