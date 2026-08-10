# Python API Reference

Primary import:

```python
import fullbleed
```

`fullbleed` re-exports the Rust extension API plus license helpers in `python/fullbleed/__init__.py`.

## `fullbleed.ui` (component-first authoring)

Secondary import surface for component/document authoring:

```python
from fullbleed.ui import el, to_html
from fullbleed.ui.core import Document
```

Key modules:

- `fullbleed.ui.core`: `Element`, `DocumentArtifact`, `Document`, `to_html`, `mount_component_html`
- `fullbleed.ui.primitives`: engine-safe layout/presentation primitives
- `fullbleed.ui.style`: inline style composition (`Style`, `style(...)`)
- `fullbleed.ui.accessibility`: semantic/a11y wrappers + `A11yContract` validator

For the accessibility-first authoring workflow (semantic tables, field grids,
signature semantics, and validation), see `docs/ui-accessibility.md`.

## `fullbleed.accessibility` (runtime/output accessibility stack)

Runtime surface for PDF/UA-targeted rendering workflows and accessibility
artifact emission:

```python
from fullbleed.accessibility import AccessibilityEngine
```

Key behavior:

- wraps `PdfEngine` with an accessibility-focused configuration surface
- emits HTML/CSS/PDF bundles with audit artifacts (`render_bundle(...)`)
- can emit engine verifier + PMR reports and PDF/UA seed checks
- emits non-visual traces (reading-order / structure) for CI and manual review support

This is the recommended runtime surface for accessibility-first projects created
with `fullbleed new accessible`.

## Main classes and helpers

## `PdfEngine`

Main render entrypoint.

Common constructor options:

- page geometry: `page_width`, `page_height`, `margin`, `page_margins`
- rendering toggles: `reuse_xobjects`, `svg_form_xobjects`, `svg_raster_fallback`
- text controls: `unicode_support`, `shape_text`, `unicode_metrics`
- PDF config: `pdf_version`, `pdf_profile`, `color_space`, output intent fields
  (`pdf_profile` accepts `none`, `pdfa1a`, `pdfa1b`, `pdfa2a`, `pdfa2b`,
  `pdfa2u`, `pdfa3a`, `pdfa3b`, `pdfa3u`, `pdfa4`, `pdfa4e`, `pdfa4f`, `pdfx4`, `pdfua1`,
  `pdfua2`, `pdfvt1`, `wtpdf1r`, `wtpdf1a`, `tagged`, plus aliases such as `a`, `ua`, `vt`, `wt1r`, and `wt1a`)
  `pdfa4`, `pdfa4e`, `pdfa4f`, `pdfua2`, `wtpdf1r`, and `wtpdf1a` emit PDF 2.0 automatically.
  PDF/A, PDF/X/VT, PDF/UA, and WTPDF text output requires embeddable font assets.
  Use `tools/validate_pdf_profiles.py` for the repeatable external/internal
  profile gate; `inspect_pdf()` exposes profile markers, seed blockers,
  embedded font counts, PDF/UA structure markers, and granular PDF/VT DPart
  graph markers. The harness includes a supplemental multipage PDF/VT specimen
  for `/Start` and `/End` range evidence.
- document metadata: `document_lang`, `document_title`
- page template decorations: header/footer text and HTML variants
- watermark controls: `watermark_*` fields or `watermark=WatermarkSpec(...)`
- diagnostics: `jit_mode`, `debug/debug_out`, `perf/perf_out`
- paginated substitutions: `paginated_context={"key": "op"}`

Key methods:

- `register_bundle(bundle)`
- `compile_pdf(html, css) -> CompiledDocument`
- `render_pdf(html, css, deterministic_hash=None) -> bytes`
- `render_pdf_to_file(html, css, path, deterministic_hash=None) -> int`
- `render_pdf_with_page_data(html, css) -> (bytes, dict|None)`
- `render_pdf_with_page_data_and_glyph_report(html, css) -> (bytes, dict|None, list[dict])`
- `plan_template_compose(html, css, templates, dx=0.0, dy=0.0) -> dict`
- `render_pdf_with_glyph_report(html, css) -> (bytes, list[dict])`
- `render_pdf_with_page_data_and_template_bindings_and_glyph_report(html, css) -> (bytes, dict|None, list[dict]|None, list[dict])`
- `render_image_pages(html, css, dpi=150) -> list[bytes]`
- `render_image_pages_to_dir(html, css, out_dir, dpi=150, stem=None) -> list[str]`
- `render_finalized_pdf_image_pages(pdf_path, dpi=150) -> list[bytes]`
- `render_finalized_pdf_image_pages_to_dir(pdf_path, out_dir, dpi=150, stem=None) -> list[str]`
- batch APIs:
  - `render_pdf_batch(..., deterministic_hash=None)`
  - `render_pdf_batch_to_file(..., deterministic_hash=None)`
  - `render_pdf_batch_with_css(..., deterministic_hash=None)`
  - `render_pdf_batch_with_css_to_file(..., deterministic_hash=None)`
  - `render_pdf_batch_parallel(..., deterministic_hash=None)`
  - `render_pdf_batch_to_file_parallel(..., deterministic_hash=None)`
  - `render_pdf_batch_to_file_parallel_with_page_data(..., deterministic_hash=None)`

`deterministic_hash` writes SHA-256 of the produced PDF bytes to the given file path.

## `CompiledDocument`

`PdfEngine.compile_pdf(html, css)` now lowers one template into two complementary programs:

- a fixed-point paint/link program for immutable copies and fixed-geometry text overlays; and
- a flow compiler that retains the recovered template tree, binding programs, CSS/page/font state,
  guarded structural flow variants, and cached PDF page-paint programs.

The reflow API described below ships in Fullbleed 2.2.4. The fixed paint/link methods remain
unchanged.

```python
compiled = engine.compile_pdf(html, css)
pdf = compiled.render_pdf()
print(compiled.stats())

# One ordered PDF containing 1,000 identical compiled copies. Untagged output
# virtualizes each source page to one shared content stream.
print_run = compiled.render_pdf_batch(1_000)

# Distinct fixed-geometry records. Every column must have the same non-zero
# length, and its key must match a {{slot_name}} found in compiled text.
invoice_template = engine.compile_pdf(
    "<p>Invoice: {{invoice_id}}</p><p>Customer: {{customer}}</p>",
    "body { font-family: Helvetica, sans-serif; }",
)
records = {
    "invoice_id": ["INV-0001", "INV-0002", "INV-0003"],
    "customer": ["Ada", "Grace", "Katherine"],
}
variable_pdf = invoice_template.render_pdf_bindings(records)
invoice_template.render_pdf_bindings_to_file(records, "invoices.pdf")

# Content-driven records. Ordinary {{slot}} values have literal-DOM semantics.
# Encountered flow shapes compile on demand; matching later records bind and
# execute guarded fixed-point programs without repeating layout.
reflow_css = "body { font-family: Helvetica, sans-serif; }"
reflow_template = engine.compile_pdf(
    """
    <article>
      <h1>{{account_name}}</h1>
      <div class="narrative">{{narrative}}</div>
      <table>
        <thead><tr><th>Item</th><th>Amount</th></tr></thead>
        <tbody data-fb-bind-html="rows"></tbody>
      </table>
    </article>
    """,
    reflow_css,
)
reflow_records = {
    "account_name": ["North", "South"],
    "narrative": ["Short review.", "A much longer review that may wrap or paginate."],
    # Explicit structural slots are trusted HTML. They may change child count
    # and element structure; use only application-generated markup here.
    "rows": [
        "<tr><td>A</td><td>$10</td></tr>",
        "<tr><td>B</td><td>$20</td></tr><tr><td>C</td><td>$30</td></tr>",
    ],
}
reflow_template.render_pdf_reflow_bindings_to_file(
    reflow_records, "reflow-records.pdf"
)
```

Methods:

- `stats() -> dict` with fixed-program `page_count`, `command_count`, `compile_ms`,
  `binding_slot_count`, and sorted `binding_slots`, plus `reflow_program_ready`,
  `reflow_program_error`, `reflow_binding_slot_count`, sorted `reflow_binding_slots`,
  `reflow_program_node_count`, `reflow_program_binding_text_node_count`, and
  `reflow_program_html_binding_node_count`
- `render_pdf(deterministic_hash=None) -> bytes`
- `render_pdf_to_file(path, deterministic_hash=None) -> int`
- `render_pdf_batch(copies, deterministic_hash=None) -> bytes`
- `render_pdf_bindings(bindings, deterministic_hash=None) -> bytes`
- `render_pdf_bindings_to_file(bindings, path, deterministic_hash=None) -> int`
- `render_pdf_reflow_bindings(bindings, deterministic_hash=None) -> bytes`
- `render_pdf_reflow_bindings_to_file(bindings, path, deterministic_hash=None) -> int`

`render_pdf_batch` is a fixed-copy virtualization API, not a dynamic template-binding API. Each
page dictionary is distinct and ordered, while identical untagged page content/resources are
linked once and referenced by every copy. Tagged profiles deliberately use page-specific streams
so their structure-parent records remain correct. A compiled object is immutable and may be
rendered concurrently from multiple Python threads.

`render_pdf_bindings` is a compiled fixed-geometry variable-data API. Parsing, selector matching,
layout, pagination, and static page paint run once. The linker shares the static page stream and
writes one compact text overlay stream for every record page. It does not copy one immutable page:
each binding row produces distinct PDF text content.

The current binding contract is deliberately narrow:

- slot markers use `{{name}}`, where `name` is at most 64 ASCII letters, digits, `_`, `-`, or `.`;
- a marker may be embedded in an ordinary text run such as `Invoice: {{invoice_id}}`;
- the mapping must contain exactly every compiled slot, and all columns must have equal non-zero
  lengths;
- slots must lower to page-local WinAnsi text outside form XObjects; immutable page-space
  transforms and rectangular/path clips are compiled into the dynamic overlay program, while
  tagged PDF profiles are not accepted by this fast path;
- values replace paint text only. They do not trigger shaping or reflow, so templates must reserve
  sufficient geometry and should currently use WinAnsi-compatible values.

Use `render_pdf_reflow_bindings` when a value can change line wrapping, element dimensions,
pagination, or complex-script shaping. It preserves the parsed/recovered template blueprint and
compiles encountered structural/input shapes into guarded fixed-point flow programs. A matching
record binds its text directly, validates width/spacing/alignment and parent-fit constraints,
shapes dynamic glyph runs, executes a cached PDF page-paint program, and compresses its page stream
on the worker. If no candidate is safe, one worker materializes its private DOM and runs full
layout/fragmentation/pagination once to add a program variant.

Execution uses native Rust scoped threads, not Python multiprocessing, and releases the GIL. The
default worker count follows available parallelism (`FULLBLEED_THREADS` can override it), the
in-flight record window is four times the worker count with a hard bound of 256, and the ordered
linker flushes at most 512 pending flow pages at a time. Completion order therefore cannot change
record/page order or deterministic bytes.

The reflow binding contract is:

- ordinary `{{name}}` markers are valid only in body text nodes. Values are literal text: `<`, `&`,
  and other untrusted characters cannot create markup;
- an empty element may opt into application-generated markup with
  `data-fb-bind-html="name"`. This trusted structural value replaces the element's children and may
  contain paragraphs, table rows, options, or other context-appropriate HTML;
- structural targets must be inside the document body, empty apart from whitespace/comments, and
  cannot be `script` or `style`; the reserved compiler-root attribute is rejected inside supplied
  markup;
- the mapping must contain exactly every reflow slot, and all columns must have equal non-zero
  lengths;
- a record executes full layout only when it cannot safely instantiate an existing flow variant.
  The compiler does not yet use a dependency DAG to limit that miss to the smallest affected
  subtree, and it does not virtualize repeated variable rows;
- each record has the same semantics as rendering the fully substituted HTML through the ordinary
  API. The compiler may therefore produce a different page count for every row.

Use `render_pdf_bindings` for validated paint-only fields where hundreds of thousands of pages per
second matter and geometry is fixed. Use `render_pdf_reflow_bindings` for compiled template reuse
with genuine content-driven pagination. Use the ordinary HTML/batch renderer for dynamic
attributes, selectors, document metadata, or other template changes outside these binding
contracts. Direct-to-file methods use buffered writers and flush before returning.

Compiled flow pages of at least 4 KiB default to the deterministic four-step Deflate search used by
the throughput lane; smaller streams retain the compact 64-step encoder. Set
`FULLBLEED_COMPILED_FLOW_DEFLATE_CHAIN=64` before process start when smaller large-page output is
more important; on the independent 1,750-page workload that setting also produced exact ordinary
renderer bytes. Values are clamped to 1-64 and are process-wide after first use.

## `AssetBundle`

Container for CSS/font/image/PDF/SVG assets.

- `add(asset)`
- `add_file(path, kind, name=None, trusted=False, remote=False)`
- `css() -> str`
- `assets_info() -> list[dict]`

## `AssetKind`

Class attributes:

- `AssetKind.Css`
- `AssetKind.Font`
- `AssetKind.Image`
- `AssetKind.Pdf`
- `AssetKind.Svg`
- `AssetKind.Other`

`Asset.info()` includes kind-specific metadata:
- `font`: primary font name (font assets)
- `pdf_version`, `page_count`, `encrypted` (PDF assets)
- `composition_supported`, `composition_issues` (PDF assets)

## `WatermarkSpec`

Constructor:

```python
fullbleed.WatermarkSpec(
    kind,
    value,
    layer="overlay",
    semantics=None,
    opacity=0.15,
    rotation_deg=0.0,
    font_name=None,
    font_size=None,
    color=None,
)
```

`kind` is one of: `text`, `html`, `image`.

## Helper functions

- `vendored_asset(source, kind, name=None, trusted=False, remote=False)`
- `inspect_pdf(path) -> dict`
- `inspect_template_catalog(templates) -> dict`
- `fetch_asset(url) -> bytes`
- `concat_css(parts: list[str]) -> str`
- `finalize_stamp_pdf(template, overlay, out, page_map=None, dx=0.0, dy=0.0) -> dict`
- `finalize_compose_pdf(templates, plan, overlay, out) -> dict`

## Component-driven project pattern

For component-style reporting:

1. Keep components in `components/`
2. Keep CSS close to each component (component styles) and compose explicitly
3. Use a report entry module that builds HTML and CSS deterministically
4. Render through `PdfEngine` from that entrypoint

See scaffold template docs in `python/fullbleed_cli/scaffold_templates/init/SCAFFOLDING.md`.
