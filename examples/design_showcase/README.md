# Designed document showcase

Four fictional document families with complete Python, HTML/CSS, data, and font
sources. [Open the published PDFs and previews](https://docs.fullbleed.dev/examples/).

| Example | Output | Design and rendering features |
| --- | --- | --- |
| Northstar Studio invoice | 1 page | Three embedded typefaces, grid, ruled line items, decimal totals, a high-contrast total panel |
| Common Ground review | 3 pages | Editorial cover, native SVG landscape, gradient chart bars, data-driven allocation shares, explicit page composition |
| Riverton service notice | 1 page | Cobalt palette, prominent schedule, semantic headings and lists, `pdfua1` profile |
| Hillside member statements | 3 records / 3 pages | Serif typography, a lilac balance panel, contribution charts, compiled reflow bindings |

## Run

From the repository root, with Python 3.10 or newer:

```bash
python -m pip install fullbleed==2.4.0
python examples/design_showcase/render.py --out output/design-showcase
```

The published examples were generated with the public 2.4.0 wheel. The script also
runs against the installed wheel in CI. No additional Python packages are needed.
Use `--only invoice`, `--only report`, `--only notice`, or `--only statements` to
render one family. PNG previews default to 120 DPI; change this with `--dpi`.

Each output directory contains a PDF, expanded HTML/CSS, PNG previews, and JSON
inspection results. The statements directory also contains its binding data and
compiled-template statistics. `verification.json` records the engine version,
font hashes, PDF hashes, page counts, and the checks performed.

## Make it yours

Edit `data.json` for invoice items, report metrics and allocations, and member
records. Edit each HTML/CSS pair for the page composition. Shared page dimensions,
typographic helpers, grids, and table rules live in `common.css`. Invoice totals
use Python `Decimal`; report allocation shares and chart heights come from data.

These are deliberately composed A4 pages with fixed page budgets. The report has
three explicit pages; it is not an arbitrary-length report template. Review every
page after adding content, changing fonts, or increasing the number of rows.

The statements use `render_pdf_reflow_bindings_to_file`, with one record per page
for this dataset. They use solid panel and chart fills for the compiled reflow
path. The report demonstrates gradients through the ordinary HTML/CSS path.

## Fonts and artwork

Inter is explicitly loaded from the installed Fullbleed wheel. Bebas Neue and DM
Serif Display are included in `fonts/`, together with their SIL Open Font License
texts. `fonts/sources.json` records the pinned upstream URLs and SHA-256 hashes.
The landscape is an editable vector drawing in `landscape.svg`.

The published previews are rendered from the finalized PDFs. Names, addresses,
organizations, financial figures, and service information are fictional.

## Verification scope

The script checks expected page counts, required text and record order, internal
PDF inspection, and matching PDF hashes on replay. The ordinary examples and a
first-record statement proof also pass the installed CLI's overflow,
missing-glyph, and font-substitution gates. The final compiled statement pages
are checked for content and replay consistency and must be visually reviewed.
CLI overflow checks do not establish that two elements never overlap.

The notice uses `pdfua1` and includes tags, language, and profile metadata. These
checks do **not** establish complete accessibility or independent standards
conformance. Validate the exact delivered artifact with the applicable automated
and human checks before making such a claim.

## One design, two authoring paths

Northstar and Common Ground also have an experimental component version:

- `invoice_components.py`: `InvoiceField`, `InvoiceItems`, `AmountDue`, and the complete invoice.
- `report_components.py`: reusable report pages, `Metric`, `ParticipationChart`, and `AllocationTable`.
- `component_primitives.py`: shared line breaks, folios, SVG asset loading, and document setup.

The component functions author content in Python with `DataTable`/`Column`, child
composition and named styling parts. They share `common.css`, the family CSS,
`data.json`, vector artwork and explicitly registered fonts with the original HTML
path. `class_name="kicker"` attaches a class; its appearance comes from the shared
`.kicker` rule passed to `Document(styles=...)`. `style=` changes a root;
`.with_parts(...)` changes the declared internal targets. No Bootstrap stylesheet
is needed for this custom design.

Install the opt-in alpha in a separate environment and compare both paths:

```sh
python -m pip install -r examples/design_showcase/requirements-components.txt
python examples/design_showcase/compare_authoring.py --out target/showcase-comparison
```

The comparison renders and verifies both versions with the same installed engine.
It requires identical page PNG bytes at 120 DPI, checks one invoice page and three
report pages, retains overflow/font diagnostics, and checks deterministic replay.
`--only invoice` or `--only report` limits the comparison. Both versions remain fixed
A4 compositions; substantial content growth needs a layout adjustment.

The checked 2.6.0a4 result has identical invoice PDF bytes. All four page images
match; Common Ground's PDF bytes differ because component-authored SVG nodes carry
extra graphics-state scopes. Its page appearance also matches under independent
PDFium rendering at 144 DPI. This compares the two paths on 2.6.0a4, not byte identity
with the historical 2.4.0 website downloads.

Use the components directly from this directory:

```python
from pathlib import Path
from fullbleed.ui.experimental import Renderer
from invoice_components import northstar_invoice
from render import DATA, FONTS

invoice = northstar_invoice(DATA["invoice"])
renderer = Renderer(font_files=[str(path) for path in FONTS])
Path("invoice.pdf").write_bytes(invoice.render_pdf(renderer))
Path("invoice.html").write_text(invoice.to_html(include_styles=True), encoding="utf-8")
Path("invoice.unstyled.html").write_text(invoice.to_html(include_styles=False), encoding="utf-8")
```

Styled export embeds CSS; unstyled export retains the authored classes and content
while omitting styles. Both are static HTML. Browser use also needs the named fonts
made available (for example through your application's `@font-face` rules); explicit
PDF font registration is separate. The comparison also verifies that direct component
rendering and the embedded-style HTML produce the same component PDF bytes.

For a small customization without editing HTML:

```python
from fullbleed.ui.experimental import pt, style
from report_components import Metric

reach = Metric(1240, "households took part", "across four neighborhoods")
accented = reach.with_parts(value=style(color="#b74624", font_size=pt(44)))
```

The experimental API may change before stabilization. Keep the exact prerelease
pin while reproducing these examples.

## Edit with a live PDF preview

After installing `requirements-components.txt`, start the local preview:

```sh
python examples/design_showcase/preview.py
```

Open the loopback URL printed in the terminal. Save `invoice_components.py`,
`component_primitives.py`, `invoice.css`, or `data.json` in your editor. The helper
reruns Python in a fresh process and refreshes images rendered from the finalized
PDF. Download the PDF, styled HTML, or unstyled HTML from the preview.

Choose Common Ground or the HTML/CSS authoring path with CLI options:

```sh
python examples/design_showcase/preview.py --document report
python examples/design_showcase/preview.py --document invoice --authoring html
```

The HTML path watches the same data and CSS, plus the original HTML template.
Its downloads contain the PDF and separate HTML/CSS sources. The preview uses
the same installed engine and fonts as the comparison command above.

Syntax errors, missing files and failed renders appear in the viewer. The last
successful preview remains visible and its downloads are explicitly labeled as
the previous version. Saving a correction retries automatically. Edits made
during a render schedule another rebuild before the result is shown. When a
document becomes shorter, the old page images disappear from the current view.

The helper watches this example directory, skipping `output`, `target`, virtual
environments and common caches. Use repeatable `--watch-path path` options for
imports or assets outside it. `--port 0` (the default) selects an available port;
`--dpi` defaults to 120, and `--timeout` allows 60 seconds per rebuild. Ctrl-C
stops the server and its render process. Download anything you need to keep:
preview artifacts live only for this session.

This is an optional local development example that executes your Python source,
binds to `127.0.0.1`, and uses only Fullbleed plus the standard library. It is not
a hosted rendering service or a replacement for the comparison/verification
command. These compositions retain their fixed page budgets; review layout
after adding content.
