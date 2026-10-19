# Headers, footers and page numbers

**Experimental API introduced in 2.6.0a9.** The earlier `2.6.0a8` wheel does
not provide these constructors. Names and contracts may change before
stabilization. Install the explicit prerelease, or build this checkout when
working from an unpublished candidate:

```console
python -m pip install --pre "fullbleed==2.6.0a9"
```

The [initial API qualification](component-page-bands-validation.md) records the
installed-wheel, saved-PDF and browser checks from implementation. The
[alpha 9 runbook](release/2.6.0a9-runbook.md) defines the new-version release gates.

Put reusable components in a `PageBand`, then attach it to `Page`. The engine
repeats the content within the page's reserved margins and supplies finalized
page values. Existing `Header` and `Footer` components remain ordinary content.

```python
from importlib.resources import files
from pathlib import Path
from fullbleed.ui.experimental import (
    Document, Heading, Page, PageBand, PageNumbers, Paragraph, Renderer,
    Theme, component, mm, pt, style,
)

@component
def Brand():
    return Paragraph("NORTHSTAR", style=style(font_size=pt(8), font_weight=700))

document = Document(
    Heading("Project summary", level=1),
    Paragraph("A reusable header and a footer with real page values."),
    title="Project summary",
    theme=Theme(text=style(font_family="Inter", font_size=pt(10))),
    page=Page(
        "A4", margin=mm(20),
        header=PageBand(left=Brand()),
        footer=PageBand(left="Project summary", right=PageNumbers(total=True)),
    ),
)
font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
Path("summary.pdf").write_bytes(document.render_pdf(Renderer(font_files=[str(font)])))
```

The complete [Common Ground field notes example](../examples/component_page_bands/README.md)
uses a shared brand component, themed content, a data table and three numbered
pages. Essential identity and report details also appear in its body.

## Slot and page ownership

`PageBand` accepts keyword-only `left`, `center` and `right` slots. Each slot
accepts the ordinary component child values: nodes, escaped text, ordered
iterables, or omitted values. Iterables are captured once. A `PageNumbers`
value occupies an entire slot; it cannot be inserted in a `Paragraph` or mixed
into a list of children.

| Setting | Default | Meaning in PDF output |
| --- | --- | --- |
| `first_page` | `True` | Repeat the band on the first page too. |
| `blank_pages` | `False` | Omit it on engine-generated blank pages, such as a blank inserted before a right-hand page. |

Suppressing a band does not change the page count. An empty body page is not
automatically a generated blank page. Named-page and facing-page variants of a
band are not part of this initial API.

`Page` retains its three positional geometry arguments. `header` and `footer`
are keyword-only additions. It still owns the paper size, orientation and
margins. A band does not grow the margins, reduce the body, or scale its content.
Neighboring populated slots are sized together by the native page-margin
layout. Rich content is laid out at its final allocated width.

An authored `@page` rule cannot also configure a margin slot owned by a band.
For example, `header=PageBand(left=...)` conflicts with an authored `@top-left`
rule, including one inside a named page, media rule or layer. Compilation raises
`AuthoringError(code="styles.page_band")`. Unowned slots remain available.

## Styles and `class_name`

Ordinary components carry their styles, resources, theme requirements and
component source locations into the band. Reusing a component in the body and a
band does not register its stylesheet twice. Slot text inherits `Theme.text`
and defaults to the slot's left, center or right alignment. A component's own
styles follow the usual [style precedence](../examples/component_recipes/README.md#reuse-styles-and-expose-customization).

A Bootstrap component brings its shared Bootstrap stylesheet into the document.
Its document-wide rules can also style body elements. Explicit component styles
and themes define the report's appearance when combining different kits.

`PageNumbers` accepts `style=` and `class_name=` on its root text box. Authored
class names connect to `Document(styles=...)`, as they do for ordinary
components. Theme defaults become generated class rules; selector styles can
override them. Explicit `style=` declarations are inline styles.

```python
footer = PageBand(right=PageNumbers(total=True, class_name="folio"))
document = Document(
    Heading("Report", level=1),
    page=Page(footer=footer),
    theme=Theme(text=style(font_family="Inter", font_size=pt(10))),
    styles={".folio": style(font_size=pt(8), color="#53665c")},
)
```

Root text and box styling, including fonts, colors, padding and backgrounds,
is carried into native finalization. `PageBand` owns positioning. Use the
`prefix`, `separator` and `suffix` arguments to author labels; generated
`::before` / `::after` content and selectors targeting the placeholder spans
are not a `PageNumbers` customization contract. Use `css(...)` for native CSS
properties outside the typed `Style` vocabulary.

Register an embedded font explicitly in `Renderer` and select that font family
in the theme or component. The band fit checker needs the actual glyph program;
it cannot prove standard, unembedded font extents by substituting another font.

## Final page values and record output

`PageNumbers()` produces `Page 1`. Set `total=True` for `Page 1 of 3`. Labels
are literal strings, escaped for both export paths:

```python
PageNumbers(total=True, prefix="", separator=" / ", suffix="")  # 1 / 3
```

Values come from completed native pagination. They are not eager `counter(page)`
spans captured when a running component is first encountered. The final string
is styled, laid out and fit-checked at its actual slot width on each page.

`Document.prepare(renderer)` returns the existing owned `CompiledDocument`.
For `render_pdf_reflow_bindings`, page numbers and totals restart for each
record. A short record and a long record can therefore have different totals
in one combined output. Changed running-content bindings require full native
reflow; no faster patching guarantee is implied.

## Fit failures and diagnostics

`Document.prepare`, `render_pdf` and `render_image_pages` raise `PageBandError`
when a native band cannot fit. The error retains `page`, `slot`, `reason`,
authored `sources` and a copied `details` payload with measurements in integer
millipoints. It is a `LayoutError` and an `AuthoringError`.

```python
from fullbleed.ui.experimental import PageBandError

try:
    document.render_pdf(renderer)
except PageBandError as error:
    print(error.page, error.slot, error.reason)
    print(error.to_dict())
```

The checker inspects both layout size and conservative paint bounds before the
outer running Form can clip the content. It uses a one-millipoint geometry
tolerance. Tight curves, outlines or stroked borders can require breathing
room even when their actual visible ink would fit. For a bordered number box,
an explicit margin such as `style(margin=pt(6), ...)` reserves that space.
Unknown paint operations fail explicitly rather than disappearing. The
[native design notes](component-page-furniture-design.md#paint-fit-prerequisite)
list the current unsupported cases.

Rendering bindings directly through `CompiledDocument` uses the runtime's
existing `ValueError` with a `fullbleed.page_margin_fit.v1` payload. A failed
batch identifies its one-based record. The component layer does not wrap an
already-returned native object. Native file/writer APIs remain streaming;
stage output when an application requires atomic replacement.

## HTML and accessibility

`to_html(include_styles=True)` exports one semantic `header`, then `main`, then
one `footer`, with collected styles. `include_styles=False` removes generated
and inline styles while preserving content and authored classes. Both exports
use explicit `[page]` and `[pages]` placeholders. They do not add JavaScript,
fabricate pagination, or promise browser-print parity.

Repeated PDF bands are artifacts outside the body reading order. Essential
content must also appear in the body; a header is not a substitute for the
document's main heading. Author the first body heading with `level=1` and use
`a11y_mode="raise"` to catch source-semantic errors before rendering.
Source semantic checks, saved-PDF inspection, visual
review and external profile validation answer different questions. A passing
fixture does not establish accessibility for every authored document or replace
human assistive-technology testing.
