<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# Fullbleed

Deterministic, dependency-free HTML/CSS-to-PDF generation in Rust, with a Python-first CLI and Python engine bindings.

## Positioning

Fullbleed is not a web-to-print runtime, and does not aim to be one.

Fullbleed is a document creation engine: HTML and CSS are used as a familiar DSL for layout, styling, and data placement in transactional documents.

HTML/CSS are the familiar document DSL; with pinned assets and flags, Fullbleed targets reproducible outputs.

This README is the canonical usage guide for:

- `fullbleed` CLI (human workflows + machine/agent automation)
- `fullbleed` Python bindings (`PdfEngine`, `AssetBundle`, batch APIs)

Additional focused references are in `docs/`:

- `docs/README.md`
- `docs/engine.md`
- `docs/python-api.md`
- `docs/cli.md`

## What You Get

- No headless browser requirement for PDF generation.
- Deterministic render pipeline with optional SHA256 output hashing.
- Reproducibility workflow via `--repro-record` and `--repro-check`.
- PDF `1.7` as the production-stable default target.
- Structured JSON result schemas for CI and AI agents.
- Offline-first asset model with explicit remote opt-in.
- Python-first extension surface for hackability and custom workflows.
- Python render calls release the GIL while Rust rendering executes.
- Rayon-backed parallelism for batch rendering and selected internal engine workloads.

## Concurrency Model

- Python binding render methods release the GIL during Rust execution (`py.allow_threads(...)` in the bridge).
- Parallel batch APIs are explicitly Rayon-backed (`render_pdf_batch_parallel(...)` and parallel-to-file variants).
- The engine also uses Rayon in selected internal hotspots (for example table layout and JIT paint paths).
- Do not assume every single-document render path will fully saturate all cores end-to-end.

## Install

```bash
python -m pip install fullbleed
```

From a local wheel:

```bash
python -m pip install C:\path\to\fullbleed-0.1.12-cp311-cp311-win_amd64.whl
```

Platform artifact policy:

- Linux (`manylinux`) and Windows wheels are built as release artifacts.
- Linux wheel builds are smoke-tested in Ubuntu/WSL during release prep.
- macOS wheel artifacts are built in CI, but are currently maintainer-untested.
- If macOS wheel behavior differs from your environment, open an issue and include `fullbleed doctor --json`.

Verify command surface:

```bash
fullbleed --help
fullbleed capabilities --json
fullbleed doctor --json
```

## 60-Second Quick Start (Project Happy Path)

Initialize project scaffold:

```bash
fullbleed init .
```

`fullbleed init` now vendors Bootstrap (`5.0.0`) into `vendor/css/bootstrap.min.css`,
vendors Bootstrap Icons (`1.11.3`) into `vendor/icons/bootstrap-icons.svg`,
vendors `inter` into `vendor/fonts/Inter-Variable.ttf`, writes license notices
(`vendor/css/LICENSE.bootstrap.txt`, `vendor/icons/LICENSE.bootstrap-icons.txt`, `vendor/fonts/LICENSE.inter.txt`),
and seeds `assets.lock.json` with pinned hashes.
The scaffolded `report.py` also runs a component mount smoke validation before
main render and writes `output/component_mount_validation.json` (fails fast on
missing glyphs, placement overflow, or CSS miss signals parsed from debug logs).
Scaffolded components now include `components/primitives.py` with reusable
layout/content helpers (`Stack`, `Row`, `Text`, table/list helpers, key/value rows, etc.).
Each scaffolded project also includes `SCAFFOLDING.md`, which should be your
first read before restructuring components.

Install additional project assets (defaults to `./vendor/...` in project context):

```bash
fullbleed assets install inter --json
```

Bootstrap baseline note:
- We target Bootstrap (`5.0.0`) as the default styling baseline for project workflows.
- Re-run `fullbleed assets install bootstrap --json` only if you want to explicitly refresh/bootstrap-manage outside `init`.

Render using the scaffolded component pipeline:

```bash
python report.py
```

Expected artifacts from scaffolded `report.py`:
- `output/report.pdf`
- `output/report_page1.png` (or equivalent page preview from engine image APIs)
- `output/component_mount_validation.json`
- `output/css_layers.json`

## Scaffold-First Workflow (Recommended)

`fullbleed init` is designed for component-first authoring rather than a single large HTML template.

Typical scaffold layout:

```text
.
|-- SCAFFOLDING.md
|-- COMPLIANCE.md
|-- report.py
|-- components/
|   |-- fb_ui.py
|   |-- primitives.py
|   |-- header.py
|   |-- body.py
|   |-- footer.py
|   `-- styles/
|       |-- primitives.css
|       |-- header.css
|       |-- body.css
|       `-- footer.css
|-- styles/
|   |-- tokens.css
|   `-- report.css
|-- vendor/
|   |-- css/
|   |-- fonts/
|   `-- icons/
`-- output/
```

Best-practice authoring model:
1. Read `SCAFFOLDING.md` first for project conventions.
2. Keep composition and data loading in `report.py`.
3. Keep reusable component building blocks in `components/primitives.py`.
4. Keep section markup in `components/header.py`, `components/body.py`, `components/footer.py`.
5. Keep component-local styles in `components/styles/*.css`.
6. Keep page tokens/composition styles in `styles/tokens.css` and `styles/report.css`.

Recommended CSS layer order:
1. `styles/tokens.css`
2. `components/styles/primitives.css`
3. `components/styles/header.css`
4. `components/styles/body.css`
5. `components/styles/footer.css`
6. `styles/report.css`

Recommended iteration loop:
1. Edit data loading + component props in `report.py`.
2. Edit component markup in `components/*.py`.
3. Edit styles in `components/styles/*.css` and `styles/*.css`.
4. Run `python report.py`.
5. Review `output/report_page1.png`, `output/component_mount_validation.json`, and `output/css_layers.json`.

Optional scaffold diagnostics:
- `FULLBLEED_DEBUG=1` to emit JIT traces.
- `FULLBLEED_PERF=1` to emit perf traces.
- `FULLBLEED_EMIT_PAGE_DATA=1` to persist page data JSON.
- `FULLBLEED_IMAGE_DPI=144` (or higher) for preview resolution.
- `FULLBLEED_VALIDATE_STRICT=1` for stricter validation gates in CI.

## One-off Quick Render (No Project Scaffold)

Render inline HTML/CSS with reproducibility artifacts:

```bash
fullbleed --json render \
  --html-str "<html><body><h1>Hello</h1></body></html>" \
  --css-str "body{font-family:sans-serif}" \
  --emit-manifest build/render.manifest.json \
  --emit-jit build/render.jit.jsonl \
  --emit-perf build/render.perf.jsonl \
  --deterministic-hash build/render.sha256 \
  --repro-record build/render.repro.json \
  --out output/hello.pdf
```

Re-run and enforce reproducibility against a stored record:

```bash
fullbleed --json render \
  --html templates/report.html \
  --css templates/report.css \
  --repro-check build/render.repro.json \
  --out output/report.rerun.pdf
```

Generate PNG page artifacts from an existing validation render:

```bash
fullbleed --json verify \
  --html templates/report.html \
  --css templates/report.css \
  --emit-pdf output/report.verify.pdf \
  --emit-image output/report_verify_pages \
  --image-dpi 200
```

Compile-only plan (no render):

```bash
fullbleed --json plan \
  --html templates/report.html \
  --css templates/report.css
```

## CLI Command Map

| Command | Purpose | JSON Schema |
| --- | --- | --- |
| `render` | Render HTML/CSS to PDF with optional PNG page artifacts | `fullbleed.render_result.v1` |
| `verify` | Validation render path with optional PDF and PNG emits | `fullbleed.verify_result.v1` |
| `plan` | Compile/normalize inputs into manifest + warnings | `fullbleed.plan_result.v1` |
| `run` | Render using Python module/file engine factory | `fullbleed.run_result.v1` |
| `compliance` | License/compliance report for legal/procurement | `fullbleed.compliance.v1` |
| `debug-perf` | Summarize perf JSONL logs | `fullbleed.debug_perf.v1` |
| `debug-jit` | Filter/inspect JIT JSONL logs | `fullbleed.debug_jit.v1` |
| `doctor` | Runtime capability and health checks | `fullbleed.doctor.v1` |
| `capabilities` | Machine-readable command/engine capabilities | `fullbleed.capabilities.v1` |
| `assets list` | Installed and optional remote packages | `fullbleed.assets_list.v1` |
| `assets info` | Package details + hashes/sizes | `fullbleed.assets_info.v1` |
| `assets install` | Install builtin/remote package | `fullbleed.assets_install.v1` |
| `assets verify` | Validate package and optional lock constraints | `fullbleed.assets_verify.v1` |
| `assets lock` | Write/update `assets.lock.json` | `fullbleed.assets_lock.v1` |
| `cache dir` | Cache location | `fullbleed.cache_dir.v1` |
| `cache prune` | Remove old cached packages | `fullbleed.cache_prune.v1` |
| `init` | Initialize project scaffold | `fullbleed.init.v1` |
| `new` | Create starter template files | `fullbleed.new_template.v1` |

Schema discovery for any command/subcommand:

```bash
fullbleed --schema render
fullbleed --schema assets verify
```

## CLI Flags That Matter Most

Global machine flags:

- `--json`: structured result payload to stdout
- `--json-only`: implies `--json` and `--no-prompts`
- `--schema`: emit schema definition and exit
- `--no-prompts`: disable interactive prompts
- `--config`: load defaults from a config file
- `--log-level error|warn|info|debug`: control CLI log verbosity
- `--no-color`: disable ANSI color output
- `--version`: print CLI version and exit

Render/verify/plan key flags:

- Inputs: `--html`, `--html-str`, `--css`, `--css-str`
  `--html` accepts `.svg` files for direct SVG-document rendering; `--html-str` accepts inline SVG markup.
- Page setup: `--page-size`, `--page-width`, `--page-height`, `--margin`, `--page-margins`
- Engine toggles: `--reuse-xobjects`, `--svg-form-xobjects`, `--svg-raster-fallback`, `--shape-text`, `--unicode-support`, `--unicode-metrics`
- PDF/compliance: `--pdf-version`, `--pdf-profile`, `--color-space`, `--document-lang`, `--document-title`
  Stable default is `--pdf-version 1.7` for shipping workflows.
  Output intent metadata (`--output-intent-identifier|--output-intent-info|--output-intent-components`) requires `--output-intent-icc`.
- Watermarking: `--watermark-text`, `--watermark-html`, `--watermark-image`, `--watermark-layer`, `--watermark-semantics`, `--watermark-opacity`, `--watermark-rotation`
- Artifacts: `--emit-jit`, `--emit-perf`, `--emit-glyph-report`, `--emit-page-data`, `--emit-image`, `--image-dpi`, `--deterministic-hash`
- Assets: `--asset`, `--asset-kind`, `--asset-name`, `--asset-trusted`, `--allow-remote-assets`
- Profiles: `--profile dev|preflight|prod`
- Fail policy: `--fail-on overflow|missing-glyphs|font-subst|budget`
- Fallback policy: `--allow-fallbacks` (keeps fallback diagnostics, but does not fail `missing-glyphs` / `font-subst` gates)
- Reproducibility: `--repro-record <path>`, `--repro-check <path>`
- Budget thresholds: `--budget-max-pages`, `--budget-max-bytes`, `--budget-max-ms`
- Release gates: `doctor --strict`, `compliance --strict --max-audit-age-days <n>`
- Commercial attestation (compliance): `--license-mode commercial`, `--commercial-licensed`, `--commercial-license-id`, `--commercial-license-file`

## SVG Workflows

Fullbleed supports SVG in three practical CLI paths:

- Direct SVG document render via `--html <file.svg>`
- Inline SVG markup via `--html-str "<svg ...>...</svg>"`
- Referenced SVG assets via `--asset <file.svg>` (kind auto-infers to `svg`)

Standalone SVG file to PDF:

```bash
fullbleed --json render \
  --html artwork/badge.svg \
  --out output/badge.pdf
```

Inline SVG markup to PDF:

```bash
fullbleed --json render \
  --html-str "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='80'><rect width='200' height='80' fill='#0d6efd'/><text x='16' y='48' fill='white'>Hello SVG</text></svg>" \
  --out output/inline-svg.pdf
```

HTML template with explicit SVG asset registration:

```bash
fullbleed --json render \
  --html templates/report.html \
  --css templates/report.css \
  --asset assets/logo.svg \
  --asset-kind svg \
  --out output/report.pdf
```

SVG render behavior flags:

- `--svg-form-xobjects` / `--no-svg-form-xobjects`
- `--svg-raster-fallback` / `--no-svg-raster-fallback`

Machine discovery:

```bash
fullbleed capabilities --json
```

Inspect the `svg` object in `fullbleed.capabilities.v1` for SVG support metadata.

## Per-Page Templates (`page_1`, `page_2`, `page_n`)

Fullbleed uses ordered page templates internally. In docs, this is easiest to think of as:

- `page_1`: first page template
- `page_2`: second page template
- `page_n`: repeating template for later pages

Configuration mapping:

- CLI `--page-margins` keys: `1`, `2`, ... and optional `"n"` (or `"each"` alias).
- Python `PdfEngine(page_margins=...)`: same key model.
- Missing numeric pages fall back to the base `margin`.
- The last configured template repeats for remaining pages.

Minimal CLI example:

```json
{
  "1": {"top": "12mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},
  "2": {"top": "24mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},
  "n": {"top": "30mm", "right": "12mm", "bottom": "12mm", "left": "12mm"}
}
```

```bash
fullbleed --json render \
  --html templates/report.html \
  --css templates/report.css \
  --page-margins page_margins.json \
  --header-each "Statement continued - Page {page} of {pages}" \
  --out output/report.pdf
```

Minimal Python example:

```python
import fullbleed

engine = fullbleed.PdfEngine(
    page_width="8.5in",
    page_height="11in",
    margin="12mm",
    page_margins={
        1: {"top": "12mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},  # page_1
        2: {"top": "24mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},  # page_2
        "n": {"top": "30mm", "right": "12mm", "bottom": "12mm", "left": "12mm"} # page_n
    },
    header_first="Account Statement",
    header_each="Statement continued - Page {page} of {pages}",
    footer_last="Final page",
)
```

Note:
- CLI currently exposes `--header-each` / `--footer-each` (and `--header-html-each` / `--footer-html-each`).
- For `first/last` header/footer variants (`header_first`, `header_last`, `footer_first`, `footer_last`), use the Python API.

## Asset Workflow (CLI)

List installed + available packages:

```bash
fullbleed assets list --available --json
```

Install builtin assets:

```bash
fullbleed assets install bootstrap
fullbleed assets install bootstrap-icons
fullbleed assets install noto-sans
# `@bootstrap` / `@bootstrap-icons` / `@noto-sans` are also supported aliases
```

PowerShell note:
- Quote `@` aliases (for example `"@bootstrap"`) to avoid shell parsing surprises.

Install remote asset package:

```bash
fullbleed assets install inter
```

Install to a custom vendor directory:

```bash
fullbleed assets install bootstrap --vendor ./vendor
```

Install to global cache:

```bash
fullbleed assets install noto-sans --global
```

Install common barcode fonts (license-safe defaults):

```bash
fullbleed assets install libre-barcode-128
fullbleed assets install libre-barcode-39
fullbleed assets install libre-barcode-ean13-text
```

Verify against lock file with strict failure:

```bash
fullbleed assets verify inter --lock --strict --json
```

Preview cache cleanup without deleting files:

```bash
fullbleed cache prune --max-age-days 30 --dry-run --json
```

Notes:

- Builtin packages accept both plain and `@` references (`bootstrap` == `@bootstrap`, `bootstrap-icons` == `@bootstrap-icons`, `noto-sans` == `@noto-sans`).
- Project installs default to `./vendor/` when project markers are present (`assets.lock.json`, `report.py`, or `fullbleed.toml` in CWD).
- If no project markers are found, `assets install` defaults to global cache unless `--vendor` is explicitly set.
- Do not hardcode cache paths like `%LOCALAPPDATA%/fullbleed/cache/...`; use `assets install --json` and consume `installed_to`.
- Installed assets include license files in typed vendor directories (for example `vendor/fonts/`, `vendor/css/`).
- `assets lock --add` is currently aimed at builtin package additions.
- Barcode packages in the remote registry are currently OFL-1.1 families from Google Fonts (`Libre Barcode`).
- USPS IMB fonts are not currently auto-installable via `assets install`; use local vetted font files and track licensing separately.

## Bootstrap Vendoring + Coverage

Bootstrap builtin package details:

- Package: `bootstrap` (alias: `@bootstrap`)
- Bundled version: `5.0.0`
- Asset kind: CSS (`bootstrap.min.css`)
- Default install location: `vendor/css/bootstrap.min.css` (project mode)
- License: `MIT`
- License source: `https://raw.githubusercontent.com/twbs/bootstrap/v5.0.0/LICENSE`

Bootstrap Icons builtin package details:

- Package: `bootstrap-icons` (alias: `@bootstrap-icons`)
- Bundled version: `1.11.3`
- Asset kind: SVG sprite (`bootstrap-icons.svg`)
- Default install location: `vendor/icons/bootstrap-icons.svg` (project mode)
- License: `MIT`
- License source: `https://raw.githubusercontent.com/twbs/icons/v1.11.3/LICENSE`

Transactional-document coverage status:

- `[sat]` Bootstrap is vendored and installable through the asset pipeline.
- `[sat]` Current Bootstrap preflight pass set is suitable for static transactional PDF workflows.
- `[sat]` Bootstrap CSS is consumed as an explicit asset (`--asset @bootstrap` or `AssetBundle`); external HTML `<link rel="stylesheet">` is not the execution path.
- Evidence source: `bootstrap_preflight.md` (visual pass dated `2026-02-10`).

Current `[pass]` fixtures from Bootstrap preflight:

| Feature | Status | Evidence |
| --- | --- | --- |
| `components/pagination` | [pass] | `examples/bootstrap5/out/components_pagination_component_page1.png` |
| `content/inline_styles` | [pass] | `examples/bootstrap5/out/content_inline_styles_component_page1.png` |
| `content/tables` | [pass] | `examples/bootstrap5/out/content_tables_component_page1.png` |
| `content/typography` | [pass] | `examples/bootstrap5/out/content_typography_component_page1.png` |
| `helpers/text_truncation` | [pass] | `examples/bootstrap5/out/helpers_text_truncation_component_page1.png` |
| `layout/bank_statement` | [pass] | `examples/bootstrap5/out/layout_bank_statement_component_page1.png` |
| `layout/breakpoints` | [pass] | `examples/bootstrap5/out/layout_breakpoints_component_page1.png` |
| `layout/columns` | [pass] | `examples/bootstrap5/out/layout_columns_component_page1.png` |
| `layout/containers` | [pass] | `examples/bootstrap5/out/layout_containers_component_page1.png` |
| `layout/grid` | [pass] | `examples/bootstrap5/out/layout_grid_component_page1.png` |
| `layout/gutters` | [pass] | `examples/bootstrap5/out/layout_gutters_component_page1.png` |
| `layout/layout_and_utility` | [pass] | `examples/bootstrap5/out/layout_layout_and_utility_component_page1.png` |
| `utilities/text_decoration` | [pass] | `examples/bootstrap5/out/utilities_text_decoration_component_page1.png` |
| `utilities/utilities` | [pass] | `examples/bootstrap5/out/utilities_utilities_component_page1.png` |
| `utilities/z_index` | [pass] | `examples/bootstrap5/out/utilities_z_index_component_page1.png` |

## `run` Command (Python Factory Interop)

`run` lets the CLI use a Python-created engine instance.

`report.py`:

```python
import fullbleed

def create_engine():
    return fullbleed.PdfEngine(page_width="8.5in", page_height="11in", margin="0.5in")
```

CLI invocation:

```bash
fullbleed --json run report:create_engine \
  --html-str "<h1>From run</h1>" \
  --css templates/report.css \
  --out output/report.pdf
```

Entrypoint formats:

- `module_name:factory_or_engine`
- `path/to/file.py:factory_or_engine`

## Python API Quick Start

```python
import fullbleed

engine = fullbleed.PdfEngine(
    page_width="8.5in",
    page_height="11in",
    margin="0.5in",
    pdf_version="1.7",
    pdf_profile="none",
    color_space="rgb",
)

html = "<html><body><h1>Invoice</h1><p>Hello.</p></body></html>"
css = "body { font-family: sans-serif; }"

bytes_written = engine.render_pdf_to_file(html, css, "output/invoice.pdf")
print(bytes_written)
```

Register local assets with `AssetBundle`:

```python
import fullbleed

bundle = fullbleed.AssetBundle()
bundle.add_file("vendor/css/bootstrap.min.css", "css", name="bootstrap")
bundle.add_file("vendor/fonts/Inter-Variable.ttf", "font", name="inter")

engine = fullbleed.PdfEngine(page_width="8.5in", page_height="11in")
engine.register_bundle(bundle)
engine.render_pdf_to_file("<h1>Styled</h1>", "", "output/styled.pdf")
```

## Python API Signatures (Runtime-Verified)

These signatures are verified from the installed package via `inspect.signature(...)`.

`PdfEngine` constructor:

```python
PdfEngine(
    page_width=None,
    page_height=None,
    margin=None,
    page_margins=None,
    font_dirs=None,
    font_files=None,
    reuse_xobjects=True,
    svg_form_xobjects=False,
    svg_raster_fallback=False,
    unicode_support=True,
    shape_text=True,
    unicode_metrics=True,
    pdf_version=None,
    pdf_profile=None,
    output_intent_icc=None,
    output_intent_identifier=None,
    output_intent_info=None,
    output_intent_components=None,
    color_space=None,
    document_lang=None,
    document_title=None,
    header_first=None,
    header_each=None,
    header_last=None,
    header_x=None,
    header_y_from_top=None,
    header_font_name=None,
    header_font_size=None,
    header_color=None,
    header_html_first=None,
    header_html_each=None,
    header_html_last=None,
    header_html_x=None,
    header_html_y_from_top=None,
    header_html_width=None,
    header_html_height=None,
    footer_first=None,
    footer_each=None,
    footer_last=None,
    footer_x=None,
    footer_y_from_bottom=None,
    footer_font_name=None,
    footer_font_size=None,
    footer_color=None,
    watermark=None,
    watermark_text=None,
    watermark_html=None,
    watermark_image=None,
    watermark_layer="overlay",
    watermark_semantics="artifact",
    watermark_opacity=0.15,
    watermark_rotation=0.0,
    watermark_font_name=None,
    watermark_font_size=None,
    watermark_color=None,
    paginated_context=None,
    jit_mode=None,
    debug=False,
    debug_out=None,
    perf=False,
    perf_out=None,
)
```

Module exports:

- `PdfEngine`
- `AssetBundle`
- `Asset`
- `AssetKind`
- `WatermarkSpec(kind, value, layer='overlay', semantics=None, opacity=0.15, rotation_deg=0.0, font_name=None, font_size=None, color=None)`
- `concat_css(parts)`
- `vendored_asset(source, kind, name=None, trusted=False, remote=False)`
- `fetch_asset(url)`

`PdfEngine` methods:

| Method | Return shape |
| --- | --- |
| `register_bundle(bundle)` | `None` |
| `render_pdf(html, css)` | `bytes` |
| `render_pdf_to_file(html, css, path)` | `int` (bytes written) |
| `render_pdf_with_glyph_report(html, css)` | `(bytes, list)` |
| `render_pdf_with_page_data(html, css)` | `(bytes, page_data_or_none)` |
| `render_pdf_batch(html_list, css)` | `bytes` |
| `render_pdf_batch_parallel(html_list, css)` | `bytes` |
| `render_pdf_batch_to_file(html_list, css, path)` | `int` |
| `render_pdf_batch_to_file_parallel(html_list, css, path)` | `int` |
| `render_pdf_batch_to_file_parallel_with_page_data(html_list, css, path)` | `(bytes_written, page_data_list)` |
| `render_pdf_batch_with_css(jobs)` | `bytes` |
| `render_pdf_batch_with_css_to_file(jobs, path)` | `int` |

`AssetBundle` methods:

- `add_file(path, kind, name=None, trusted=False, remote=False)`
- `add(asset)`
- `assets_info()`
- `css()`

## Python Examples (Smoke-Checked)

Text watermark + diagnostics:

```python
import fullbleed

engine = fullbleed.PdfEngine(
    page_width="8.5in",
    page_height="11in",
    margin="0.5in",
    pdf_version="1.7",
    watermark_text="INTERNAL",
    watermark_layer="overlay",
    watermark_semantics="artifact",
    watermark_opacity=0.12,
    watermark_rotation=-32.0,
    debug=True,
    debug_out="build/invoice.jit.jsonl",
    perf=True,
    perf_out="build/invoice.perf.jsonl",
)

html = "<h1>Invoice</h1><p>Status: Ready</p>"
css = "h1{margin:0 0 8px 0} p{margin:0}"

written = engine.render_pdf_to_file(html, css, "output/invoice_watermarked.pdf")
print("bytes:", written)
```

Batch render + glyph/page-data checks:

```python
import fullbleed

engine = fullbleed.PdfEngine(page_width="8.5in", page_height="11in", margin="0.5in")

jobs = [
    ("<h1>Batch A</h1><p>Alpha</p>", "h1{color:#0d6efd}"),
    ("<h1>Batch B</h1><p>Beta</p>", "h1{color:#198754}"),
]

written = engine.render_pdf_batch_with_css_to_file(jobs, "output/batch.pdf")
print("batch bytes:", written)

pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report("<p>Hello</p>", "")
print("glyph entries:", len(glyph_report))

pdf_bytes, page_data = engine.render_pdf_with_page_data("<p>Hello</p>", "")
print("page data available:", page_data is not None)
```

## Transactional Header/Footer + Totals

Minimal, self-contained Python example (no external template files) showing:

- Continued headers on page 2+.
- Per-page subtotal footer expansion via `{sum:items.amount}`.
- Final-page grand total footer expansion via `{total:items.amount}`.
- Structured `page_data` totals for automation and reconciliation checks.

```python
from pathlib import Path
import fullbleed

rows = []
for i in range(1, 121):  # enough rows to force multiple pages
    amount = 10.00 + ((i * 7) % 23) + 0.25
    rows.append(
        f'<tr data-fb="items.amount={amount:.2f}">'
        f"<td>2026-01-{(i % 28) + 1:02d}</td>"
        f"<td>Txn {i:03d}</td>"
        f'<td class="num">${amount:.2f}</td>'
        "</tr>"
    )

html = f"""<!doctype html>
<html>
<body>
  <h1>Monthly Statement</h1>
  <table>
    <thead>
      <tr><th>Date</th><th>Description</th><th class="num">Amount</th></tr>
    </thead>
    <tbody>
      {''.join(rows)}
    </tbody>
  </table>
</body>
</html>
"""

css = """
body { font-family: sans-serif; font-size: 10pt; color: #111; }
h1 { margin: 0 0 8pt 0; }
table { width: 100%; border-collapse: collapse; }
th, td { padding: 4pt; border-bottom: 1pt solid #e1e1e1; }
thead th { background: #f3f6fa; text-transform: uppercase; font-size: 9pt; }
.num { text-align: right; }
"""

engine = fullbleed.PdfEngine(
    page_width="8.5in",
    page_height="11in",
    margin="12mm",
    page_margins={
        1: {"top": "12mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},
        2: {"top": "28mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},
        "n": {"top": "28mm", "right": "12mm", "bottom": "12mm", "left": "12mm"},
    },
    header_html_each=(
        '<div style="display:flex;justify-content:space-between;border-bottom:1pt solid #d9d9d9;">'
        '<div style="font-weight:bold;">Acme Ledger</div>'
        '<div style="font-size:9pt;color:#444;">Statement Continued - Page {page} of {pages}</div>'
        "</div>"
    ),
    header_html_x="12mm",
    header_html_y_from_top="6mm",
    header_html_width="186mm",
    header_html_height="10mm",
    paginated_context={"items.amount": "sum"},
    footer_each="Subtotal (Page {page}): ${sum:items.amount}",
    footer_last="Grand Total: ${total:items.amount}",
    footer_x="12mm",
    footer_y_from_bottom="8mm",
)

pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
Path("output_transactional_minimal.pdf").write_bytes(pdf_bytes)

assert page_data["page_count"] >= 2
assert page_data["totals"]["items.amount"]["value"] == sum(
    p["items.amount"]["value"] for p in page_data["pages"]
)
print("Wrote output_transactional_minimal.pdf")
print("Page count:", page_data["page_count"])
print("Grand total:", page_data["totals"]["items.amount"]["formatted"])
```

API note:
- For transactional running totals (`paginated_context`) and HTML header/footer placement (`header_html_*`, `footer_html_*`), use the Python `PdfEngine` API path.
- The CLI currently exposes direct text header/footer flags (`--header-each`, `--footer-each`) for simpler cases.

CLI watermark parity example:

```bash
fullbleed --json render \
  --html-str "<h1>Watermark probe</h1><p>hello</p>" \
  --css-str "body{font-family:sans-serif}" \
  --watermark-text "INTERNAL" \
  --watermark-layer overlay \
  --watermark-opacity 0.12 \
  --watermark-rotation -32 \
  --out output/watermark_probe.pdf
```

## Reference-Image Parity Workflow (Practical)

When targeting a design reference image (for example reference image exports), this loop has worked well:

1. Start from `fullbleed init` so CSS/font/icon baselines are vendored and pinned.
2. For scaffolded projects, run `python report.py` and set `FULLBLEED_IMAGE_DPI` as needed for sharper previews.
3. For direct CLI template rendering, register assets through the CLI (`--asset ...`) or `AssetBundle`.
4. Iterate with image artifacts enabled:

```bash
fullbleed --json render \
  --profile preflight \
  --html templates/invoice.html \
  --css templates/invoice.css \
  --asset vendor/css/bootstrap.min.css --asset-kind css --asset-name bootstrap \
  --asset vendor/icons/bootstrap-icons.svg --asset-kind svg --asset-name bootstrap-icons \
  --asset vendor/fonts/Inter-Variable.ttf --asset-kind font --asset-name inter \
  --emit-image output/pages_png \
  --emit-jit output/render.jit.jsonl \
  --emit-perf output/render.perf.jsonl \
  --out output/render.pdf
```

5. Use `--repro-record` / `--repro-check` once your layout stabilizes.

Practical tips:
- Compare against full-page exports when available.
- Keep a fixed preview DPI (for example `144` or `200`) across iterations.
- Commit PNG baselines for repeatable visual checks.

## Public Golden Regression Suite

Launch-grade render regression coverage is available under `goldens/` with three fixtures:

- `invoice`
- `statement`
- `menu`

Golden contract assets:

- Expected hashes: `goldens/expected/golden_suite.expected.json`
- Expected PNG baselines: `goldens/expected/png/<case>/<case>_page1.png`

Run against committed expectations:

```bash
python goldens/run_golden_suite.py verify
```

Refresh baselines intentionally:

```bash
python goldens/run_golden_suite.py generate
```

## Human + AI Operating Mode

Recommended automation defaults:

```bash
fullbleed --json-only render ...
```

Why this is agent-safe:

- For command-execution JSON payloads, `schema` is always present.
- Parser usage errors (`exit=2`) are emitted by argparse as usage text, not JSON payloads.
- `ok` indicates success/failure without parsing text.
- Optional artifacts are explicitly named in `outputs`.
- Schema introspection is available at runtime (`--schema`).

Example parse loop:

```python
import json, subprocess

proc = subprocess.run(
    [
        "fullbleed", "--json-only", "render",
        "--html", "templates/report.html",
        "--css", "templates/report.css",
        "--out", "output/report.pdf",
    ],
    capture_output=True,
    text=True,
    check=False,
)

payload = json.loads(proc.stdout)
assert payload["schema"] == "fullbleed.render_result.v1"
assert payload["ok"] is True
print(payload["outputs"]["pdf"])
```

## MACHINE_CONTRACT.v1

```json
{
  "schema": "fullbleed.readme_contract.v1",
  "package": "fullbleed",
  "cli_entrypoint": "fullbleed",
  "dev_cli_entrypoint": "python -m fullbleed_cli.cli",
  "python_module": "fullbleed",
  "json_discriminator": "schema",
  "core_commands": [
    "render",
    "verify",
    "plan",
    "debug-perf",
    "debug-jit",
    "run",
    "compliance",
    "doctor",
    "capabilities",
    "assets",
    "cache",
    "init",
    "new"
  ],
  "result_schemas": [
    "fullbleed.render_result.v1",
    "fullbleed.verify_result.v1",
    "fullbleed.plan_result.v1",
    "fullbleed.run_result.v1",
    "fullbleed.compliance.v1",
    "fullbleed.capabilities.v1",
    "fullbleed.doctor.v1",
    "fullbleed.assets_list.v1",
    "fullbleed.assets_info.v1",
    "fullbleed.assets_install.v1",
    "fullbleed.assets_verify.v1",
    "fullbleed.assets_lock.v1",
    "fullbleed.cache_dir.v1",
    "fullbleed.cache_prune.v1",
    "fullbleed.init.v1",
    "fullbleed.new_template.v1",
    "fullbleed.debug_perf.v1",
    "fullbleed.debug_jit.v1",
    "fullbleed.repro_record.v1",
    "fullbleed.error.v1"
  ],
  "artifact_flags": [
    "--emit-manifest",
    "--emit-jit",
    "--emit-perf",
    "--emit-glyph-report",
    "--emit-page-data",
    "--emit-image",
    "--image-dpi",
    "--deterministic-hash",
    "--repro-record",
    "--repro-check"
  ],
  "fail_on": ["overflow", "missing-glyphs", "font-subst", "budget"],
  "budget_flags": ["--budget-max-pages", "--budget-max-bytes", "--budget-max-ms"],
  "profiles": ["dev", "preflight", "prod"],
  "pdf_version_default": "1.7",
  "known_exit_codes": {
    "0": "success",
    "1": "command-level validation/operational failure",
    "2": "argparse usage error",
    "3": "CLI runtime/input error wrapper"
  }
}
```

## Important Behavior Notes

- `render --json` cannot be combined with `--out -` (stdout PDF bytes).
- `verify` defaults to stdout PDF unless `--emit-pdf` is provided; for machine mode, use `--emit-pdf <path>`.
- `--emit-image <dir>` writes per-page PNGs as `<stem>_pageN.png` (stem comes from `--out`/`--emit-pdf`, or `render` when streaming PDF to stdout).
- If both `--emit-page-data` and `--emit-glyph-report` are set, render is performed twice.
- Production target is PDF `1.7`.
- `run` accepts `--html-str` without requiring `--html`.
- `run` emits a one-time AGPL/commercial licensing reminder; suppress with `--no-license-warn` or by activating commercial attestation (`FULLBLEED_LICENSE_MODE=commercial` + `FULLBLEED_COMMERCIAL_LICENSED=1`).
- `init` now scaffolds `COMPLIANCE.md` for project-level release review.
- `compliance --json` emits machine-readable legal/procurement diagnostics.
- `--watermark-layer underlay` is accepted as a legacy alias and normalized to `background`.
- `--emit-manifest` includes a `watermark` object with `text|html|image|layer|semantics|opacity|rotation|enabled`.
- `--fail-on overflow` is enforced from placement data and may auto-enable internal JIT planning.
- `--fail-on font-subst` is enforced using missing glyph and fallback diagnostics.
- `--allow-fallbacks` allows fallback diagnostics to remain informational for `missing-glyphs` / `font-subst` gates while still reporting them in JSON output.
- `--fail-on budget` requires at least one budget threshold flag.
- `--repro-check` fails on input/hash drift and lock hash mismatches when lock data is available.
- `--pdf-profile pdfx4` enforces embedded-font constraints; CLI errors include an actionable hint to add an embeddable font asset.
- `argparse` usage errors exit with code `2` and emit usage text (not JSON), even when `--json` is present.

## Related Docs

- Agent workflow guide: `llm.txt`
- CLI JSON contract quick reference: `cli_schema.md`
- CLI epoch/spec: `CLI_EPOCH.md`
- Licensing guide: `LICENSING.md`
- Third-party notices: `THIRD_PARTY_LICENSES.md`
- Living docs example project: `examples/living_docs_atlas/README.md`
- Roofing invoice parity example: `examples/roofing_invoice/README.md`
- Iconography smoke example: `examples/iconography_test/README.md`
- Public golden regression suite: `goldens/README.md`

## License

Fullbleed is dual-licensed:

- Open-source option: `AGPL-3.0-only` (`LICENSE`)
- Commercial option: `LicenseRef-Fullbleed-Commercial` (`LICENSING.md`)

SPDX expression:

- `AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial`

- Copyright notice: `COPYRIGHT`
- Third-party notices: `THIRD_PARTY_LICENSES.md`
- Practical licensing guide: `LICENSING.md`

For license information, please visit `fullbleed.dev` or email `info@fullbleed.dev`.

License integrity gate (CI-friendly, no build required):

```bash
python tools/check_license_integrity.py --json
```

Commercial compliance attestation examples:

```bash
fullbleed compliance --json \
  --license-mode commercial \
  --commercial-license-id "ACME-2026-001"
```

```bash
# env-based attestation (useful in CI/containers)
set FULLBLEED_LICENSE_MODE=commercial
set FULLBLEED_COMMERCIAL_LICENSED=1
set FULLBLEED_COMMERCIAL_LICENSE_ID=ACME-2026-001
fullbleed compliance --json
```

```python
import fullbleed

# Process-local helper for library users and agent runtimes.
fullbleed.activate_commercial_license(
    "ACME-2026-001",
    company="Acme Corp",
    tier="$1,000,001-$10,000,000",
)
```

