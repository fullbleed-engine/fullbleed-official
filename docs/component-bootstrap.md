# Static Bootstrap components and HTML export

The experimental Python kit covers the 23 component families in **Bootstrap
5.0.0**, plus layout, content, forms, helpers and the classes in its compiled
utility stylesheet. It uses the repository's existing, unmodified MIT-licensed
stylesheet. Components carry that dependency; a document includes it once.

The published **2.6.0a12** alpha provides this kit under
`fullbleed.components.bootstrap`. Install it explicitly in a virtual
environment: `python -m pip install --pre "fullbleed==2.6.0a12"`.
Ordinary `pip install fullbleed` selects stable **2.5.34**, which includes the
established `fullbleed.ui` API. The new kit remains experimental; see the
[current release decision and stabilization criteria](https://docs.fullbleed.dev/guides/component-status/).
The [alpha runbook](release/2.6.0a12-runbook.md) records the release gates and scope.
For an unpublished checkout, build with
`python build_backend/fullbleed_build_backend.py wheel --out dist` and install
the printed wheel path.

Earlier `fullbleed.ui.experimental.bootstrap` imports remain supported and
expose the same kit. See [component imports and migration](component-namespace.md)
for compatibility with both earlier authoring APIs.

The [published Bootstrap preview](https://github.com/fullbleed-engine/fullbleed-official/releases/tag/component-preview-41a2cee)
supports this kit and HTML export. The alpha's newer
[part-forwarding contract](../examples/component_recipes/README.md#customize-a-root-or-an-exposed-part)
adds `forward_parts(...)` and explicit required/optional declarations. Existing
preview downloads are unchanged.

All components are static. There is no Bootstrap JavaScript, browser in the PDF
renderer, CDN dependency, event lifecycle or automatic network fetch.

## Your first PDF and HTML exports

After installing the alpha, save this as `order.py` and run `python order.py`.
It writes a PDF and both styled and unstyled HTML using the bundled Inter font:

```python
from importlib.resources import files
from pathlib import Path
from fullbleed.components import Document, Renderer, Theme, pt, style
from fullbleed.components import bootstrap as bs

card = bs.Card(
    bs.Paragraph("Your order has passed its final inspection."),
    title="Ready to ship",
    header="ORDER / 1048",
    footer=bs.Badge("Approved", variant="success"),
).with_parts(title=style(color="#205b47"))

document = Document(card, title="Order summary", lang="en-US",
                    theme=Theme(text=style(font_family="Inter", font_size=pt(11))))
Path("order.html").write_text(document.to_html(include_styles=True), encoding="utf-8", newline="\n")
Path("order.unstyled.html").write_text(document.to_html(include_styles=False), encoding="utf-8", newline="\n")
renderer = Renderer(font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))])
Path("order.pdf").write_bytes(document.render_pdf(renderer))
```

This complete example is checked from an installed wheel. The following sections
explain composition, styling and the static Bootstrap component families.

## Start with content

```python
from fullbleed.components import Document, style
from fullbleed.components import bootstrap as bs

summary = bs.Card(
    bs.Paragraph("Your order has passed its final inspection."),
    title="Ready to ship",
    header="ORDER / 1048",
    footer=bs.Badge("Approved", variant="success"),
).with_parts(
    title=style(color="#205b47"),
    body=style(padding="1.5rem"),
)

document = Document(summary, title="Order summary")
html = document.to_html(include_styles=True)
plain_html = document.to_html(include_styles=False)
```

The card owns its body, heading, header and footer markup. Its public style
parts are `header`, `body`, `title`, `subtitle` and `footer`. Overrides affect
that card's sections, preserving nested cards' own appearance. An absent
optional section remains absent. For complete structural control, pass
`body=False` and compose `CardBody`, `CardHeader`, `CardFooter`, `CardTitle`,
`CardSubtitle`, `CardText`, `CardImage` and `CardImageOverlay` yourself.

Components are immutable values. Ordinary functions, positional children,
keyword arguments, conditions, lists and generators provide composition.
`None` and booleans omit children. Strings are escaped text, so no raw HTML
strings are needed. `@component` adds a useful name and source location to
errors; it does not add a DOM wrapper.

## How styling works

Three mechanisms have distinct jobs:

| Input | Meaning | Example |
| --- | --- | --- |
| Named component options | Checked choices that build the right structure/classes | `Badge("Paid", variant="success", pill=True)` |
| `style=` and `.with_style(...)` | Explicit declarations on the component's root | `style(border_color="#b8cec4")` |
| `class_name=` | Literal CSS classes, including your own selectors | `class_name="invoice-note mb-3"` |

The Python spelling is **`class_name`**. It does not search for CSS files or
register global styles. The Bootstrap kit brings Bootstrap CSS because it is
a known component dependency. A custom name such as `invoice-note` needs a rule
in `Document(styles=...)`:

```python
document = Document(
    bs.Alert("Please retain this copy.", class_name="invoice-note"),
    styles={".invoice-note": style(color="#304f45", border_color="#b8cec4")},
)
```

`bs.classes(...)` combines strings, ordered collections and conditional mappings,
deduplicating classes in authored order. `bs.utilities(...)` checks actual class
names against the bundled 5.0.0 stylesheet:

```python
panel = bs.Box(
    "A highlighted note",
    class_name=bs.classes(
        "invoice-note",
        {"highlighted": True, "archived": False},
        bs.utilities(p=3, mt={"md": 2}, text=("primary", "center")),
    ),
)
```

Utility names use Bootstrap prefixes (`p`, `mt`, `bg`, `d`, `align_items`, etc.).
Use `True` for a bare class, such as `clearfix=True`. Responsive mappings use
`xs`/`default`, `sm`, `md`, `lg`, `xl`, `xxl`, and `print` where the compiled CSS
provides that class. Unknown classes and breakpoints fail early. Negative margin
utilities are not enabled in the shipped Bootstrap build. Sass configuration,
mixins and a Sass compiler are outside this Python API.

Component CSS comes first, then document defaults/recipes and your document
selector rules. Explicit root styles are inline declarations. Normal CSS
specificity still applies: Bootstrap utility declarations using `!important`
can override a normal inline declaration. Avoid giving the same property
conflicting utility and inline values.

Use `styled(...)` for reusable defaults and `.with_parts(...)` for a component's
public sections. `style(...)` checks a documented vocabulary of common
properties; it is not the engine's complete CSS vocabulary. For additional
declarations, compose the explicit `css(...)` helper:

```python
from fullbleed.components import css, styled

ReceiptCard = styled(bs.Card, border_color="#d2e0da", border_radius="8px")
receipt = ReceiptCard("Payment received.", title="Receipt").with_style(
    css("box-shadow: inset 4px 0 0 0 #205b47;")
).with_parts(body=style(padding="1.5rem"))
```

This returns a new component value; the reusable defaults are unchanged.
`include_styles=True` collects these declarations and Bootstrap CSS into the
HTML export. `include_styles=False` omits them while keeping the content,
structure and authored classes for the host page to style.

Set inherited document typography with `Theme(text=style(...))`. For PDF output,
register the corresponding font file with `Renderer`; a CSS family name alone
does not load a font. For example, using Fullbleed's bundled Inter font:

```python
from importlib.resources import files
from fullbleed.components import Renderer, Theme

document = Document(
    receipt,
    theme=Theme(text=style(font_family="Inter", font_size="11pt")),
)
renderer = Renderer(font_files=[
    str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")),
])
pdf = document.render_pdf(renderer)
```

The alpha 3 candidate fixes an earlier cascade bug that could make Bootstrap's
variable-based default font override this theme. Styled HTML carries the CSS
family declaration; it does not embed the renderer's font files. A browser host
must provide that family itself, for example through its own `@font-face` rule.
Unstyled HTML leaves both typography and other styling to the host.

## Layout and data tables

```python
rows = [{"item": "Studio notebook", "quantity": 24, "amount": "$360.00"}]

table = bs.DataTable(
    rows,
    columns=[
        bs.Column("Item", lambda row: row["item"]),
        bs.Column("Quantity", lambda row: row["quantity"], align="right"),
        bs.Column("Amount", lambda row: row["amount"], align="right"),
    ],
    striped=True,
    caption="Items in this shipment",
)

layout = bs.Container(
    bs.Row(
        bs.Col(bs.Card(table, title="Order details"), span=8),
        bs.Col(bs.Card("Tracked delivery", title="Shipping"), span=4),
        gutter=3,
    ),
    fluid=True,
)
```

Columns support spans 1–12, `"auto"`, or breakpoint mappings such as
`span={"xs": 12, "md": 8}`. `Row` supports responsive column counts and gutters.
`DataTable` captures rows and cell callbacks once, emits scoped column headers,
and retains the existing repeated-header and row-pagination behavior. For manual
composition use `Table`, `TableHead`, `TableBody`, `TableFoot`, `TableRow`,
`TableHeader`, `TableCell` and `TableCaption`.

## States and form labels

Group helpers derive child IDs from one explicit group ID and optional stable
item keys. IDs must be unique in the document; there is no global counter.

```python
details = bs.Accordion(
    bs.AccordionItem("Delivery", "Two working days.", key="delivery"),
    bs.AccordionItem("Returns", "Unused items only.", key="returns", open=False),
    id="order-details",
)

tabs = bs.Tabs(
    bs.Tab("Summary", "Current order details.", key="summary"),
    bs.Tab("History", "Previous orders.", key="history"),
    id="order-tabs",
    active="summary",
)

field = bs.FloatingLabel(
    bs.Input(value="Northstar Studio"), "Company", id="company",
)
check = bs.Check("Include archive copy", id="archive-copy", checked=True)
```

`Tab`, `AccordionItem` and `CarouselItem` are immutable item descriptions consumed
by their parent. They accept ordered generators as well as positional items.
`active` selects a key or a zero-based index. Accordion and collapse use `open`;
toasts, dropdowns, modals and offcanvas panels also have an authored `open` state.
Hidden content remains hidden in unstyled HTML through the `hidden` attribute.

Tabs do not switch, carousels do not advance, scrollspy does not track scrolling,
and spinners do not animate. Dropdowns, popovers, tooltips, modals and offcanvas
panels appear in document flow. They have no backdrop, focus trap or close
action; `CloseButton` is disabled by default. Links retain their destinations.
HTML forms remain ordinary HTML controls, with no application behavior added.

PDF forms are **static printed values**, not fillable PDF fields. Input and
textarea values are preserved; password values are masked; selects print their
selected option(s); checkboxes, radios, switches and ranges show authored state.
Floating labels print above their values. Navigation and button print boxes
avoid native list/control layout restrictions while retaining links and list
intent. These are explicit print representations, not a browser rendering claim.

## Export HTML or render PDF

| Call | Result |
| --- | --- |
| `document.to_html(include_styles=True)` | Complete HTML document with collected CSS in the head |
| `document.to_html(include_styles=False)` | Complete HTML without generated/inline style declarations; authored class names remain |
| `node.to_html(include_styles=True)` | Component fragment with its collected styles; no page/body typography rules |
| `node.to_html(include_styles=False)` | Unstyled component fragment |
| `to_html(node, theme=..., include_styles=True)` | Functional fragment exporter with explicit token values |
| `document.to_html()` / `document.to_css()` | Existing separate HTML/CSS behavior for compatibility |
| `document.compile(target="html")` | Source with normal HTML controls (the default) |
| `document.compile(target="pdf")` | Source with static print representations |

Styled export embeds CSS, not image or font files. Pass your own asset URLs for
browser use. HTML export never creates a `PdfEngine`. It can be used in a web
application or static-site generator; the installed Fullbleed distribution is
still required. This change does not create a separate lightweight HTML package.

```python
from importlib.resources import files
from pathlib import Path
from fullbleed.components import Renderer

font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
renderer = Renderer(font_files=[str(font)])
Path("order.pdf").write_bytes(document.render_pdf(renderer))
```

In styled HTML, `Flow` becomes a vertical flex container: its gap works in a
browser, and neighboring child margins add instead of collapsing. Recipes,
`gap=`, `.with_style(...)`, named parts and `class_name` rules retain their usual
precedence. No browser stylesheet workaround is needed. `PageBreak` markers
are hidden in this continuous HTML representation. Unstyled export omits these
layout rules along with the other styles.

`prepare`, `render_pdf` and `render_image_pages` select the print representation
automatically. To retain the exact source used for a PDF, call
`document.emit_artifacts(html_path="order.html", css_path="order.css", target="pdf")`.
`Flow` retains its native block pagination in that target. Browser layout,
including fixed-height flex behavior, is not a PDF pagination preview.
The legacy `to_legacy` bridge rejects components with stylesheet dependencies
instead of silently dropping their appearance.

## Coverage and evidence

The catalog covers these tagged Bootstrap 5.0.0 families: accordion, alerts,
badge, breadcrumb, buttons, button group, card, carousel, close button, collapse,
dropdowns, list group, modal, navs/tabs, navbar, offcanvas, pagination, popovers,
progress, scrollspy, spinners, toasts and tooltips. It also includes containers,
grid/gutters, typography, figures/images, tables, form controls, input groups,
floating labels, validation, ratios, visually hidden helpers and checked
utilities. `bs.COMPONENT_FAMILIES` and `bs.__all__` expose the inventory.

Run the [rich report and catalog](../examples/component_bootstrap/README.md).
The [validation record](component-bootstrap-validation.md) distinguishes tested
specimens from wider CSS support. Coverage of the component vocabulary does not
establish every combination's PDF fidelity, browser pixel parity, accessibility
conformance or every Bootstrap CSS feature. Responsive layouts in a PDF use the
engine's print-media/page-width behavior, not a live browser viewport.

The normative dependency is the
[Bootstrap v5.0.0 tag](https://github.com/twbs/bootstrap/tree/v5.0.0), including
its [component inventory](https://github.com/twbs/bootstrap/blob/v5.0.0/site/data/sidebar.yml)
and [compiled stylesheet](https://github.com/twbs/bootstrap/blob/v5.0.0/dist/css/bootstrap.min.css).
The current `/docs/5.0/` website also includes later 5.0 patch changes; it is not
the version lock for this kit.
