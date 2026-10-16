# Experimental component authoring

This prototype explores a document API with reusable styling, named component
parts, theme values, page geometry, and data tables. It is available from this
source branch's wheel under `fullbleed.ui.experimental`; it is not in the
published 2.5.25 wheel. Names and contracts may change before stabilization.

The [implementation review](../../docs/component-authoring-design.md) explains
the design decisions, remaining gaps, and criteria for stabilizing this API.

The [invoice and report](documents.py) use the same [design kit](kit.py). Neither
document imports a stylesheet or coordinates selectors. The kit contains the
styles and the components that use them. The example data is the existing
fictional [Northstar invoice data](../components/data.json).

## Reuse a component with its styles

```python
from fullbleed.ui.experimental import (
    Document, Paragraph, Stack, Theme, component, pt, style, styled, token,
)

Label = styled(Paragraph, font_size=pt(8), color=token("muted"))

@component(parts=("label", "value"))
def Field(label: str, value: str):
    return Stack(
        Label(label, part="label"),
        Paragraph(value, part="value"),
        gap=pt(4),
    )

field = Field("Prepared for", "Maple & Finch")
document = Document(field, theme=Theme({"muted": "#667580"}))
```

`styled` captures a reusable root style. Compilation collects the styles used by
the document and generates class names from the resolved declarations. There is
no module-level style registry or stylesheet registration step. A component using
theme tokens requires a theme that supplies those tokens; a missing value reports
the component and part that referenced it.

`@component` preserves the callable signature and records a boundary without
adding an HTML wrapper. Ordinary functions also work. Lists and ordered generators
are captured once; strings are escaped, booleans and `None` are omitted, and
unformatted mappings or uncalled functions fail.

## Customize a root or an exposed part

```python
custom = field.with_style(background="#f2f5f8", padding=pt(12))
custom = custom.with_parts(label=style(color="#315d9b", font_weight=700))
```

The function does not need to forward styling arguments. Both operations return
new nodes. A function delegating its root to another component retains that root's
exposed parts. Named parts stop at components nested inside the root, so an outer component's
`title` does not inadvertently change a nested component's `title`. Unknown part
names raise an error listing the available names. Root styling requires a single rooted element; fragments
need an explicit container.

Declare a reusable component's public parts with `@component(parts=(...))`.
Declared parts can be absent in an instance: an optional note or an empty table's
cells can receive overrides without making callers branch on the content.
Authored `part=` markers must match the declaration; misspellings fail when the
component is constructed. Nested components validate their own declarations.
An explicit declaration on a delegating wrapper can expose a subset of its root's
parts. An undeclared wrapper retains root delegation.

`Field.parts` exposes its declaration, and `field.parts` exposes the instance's
public names. Plain `@component` remains supported and discovers names from
present content; use an explicit declaration for a stable customization contract.
`ComponentFactory` preserves a decorated function's argument types while exposing
its optional declaration. Style-property and part-name completion remain future work.

Styles compose in this order: primitive defaults, a styled factory's defaults,
the call's `style`, then later `with_style` or `with_parts` changes. Overrides move
to the end, preserving CSS shorthand/longhand ordering. Themes supply values and
document text defaults; they do not silently override a component's own settings.

`class_name` still means a literal CSS class label. It does not load styles or
refer to a Python variable. In this prototype, use style values for the normal
authoring path. `css("transform: rotate(2deg)")` is an explicit declaration escape
hatch for CSS beyond the checked property vocabulary. Full selector stylesheets
remain available through the established `fullbleed.ui` and engine APIs.

## Describe table columns once

```python
from fullbleed.ui.experimental import Column, DataTable, mm

table = DataTable(
    [{"name": "Design", "amount": "$120.00"}],
    columns=[
        Column("Work", lambda row: row["name"]),
        Column("Amount", lambda row: row["amount"], align="right", width=mm(30)),
    ],
    empty="No billable items.",
).with_parts(header=style(background="#f2f5f8"), cell=style(padding=pt(10)))
```

Columns define headers, cell functions, alignment, and optional widths together.
Cells can contain components. Rows and callbacks are evaluated once during
construction. The table owns column-header scope, repeated headers, and empty
state spanning. With `repeat_header=False`, scoped column-header cells stay in
the first body row because the native engine always repeats a `thead` group.
Application code owns monetary calculations and formatting.

Table parts are `header`, `cell`, `row`, `caption`, and `empty`, whether or not the
current data produces cells, a caption, or an empty-state row. Each body cell retains its table,
row, and column path in the compiled source map.

**Pagination:** ordinary rows move intact when they fit on a fresh page.
Top-aligned oversized cells continue independently through the native layout
engine, with repeated headers and stable column widths on equal-width pages.
The examples check long text, uneven cells, borders, and mixed content.
Tables containing rowspans, page-tall images inside cells, and oversized
non-top-aligned cells still need work.
When the engine reports forced unsplittable overflow, the experimental renderer
raises `layout.unsplittable` before returning a PDF. Changing page widths and
more complex cell decorations need further qualification.

## Configure the page and render

```python
from importlib.resources import files
from pathlib import Path
from fullbleed.ui.experimental import Page, Renderer

document = Document(
    field,
    title="Customer details",
    lang="en-US",
    page=Page("A5", margin=mm(12)),
    theme=Theme({"muted": "#667580"}, text=style(font_family="Inter")),
)
renderer = Renderer(
    font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))],
    document_timestamp="2026-10-15T00:00:00Z",
)
Path("customer.pdf").write_bytes(document.render_pdf(renderer))
```

`Page` controls actual CSS page geometry. `Document` owns the PDF title and
language as well as HTML metadata. `Renderer` takes the engine's font, profile,
and diagnostic options; conflicting page or metadata options fail explicitly.
It creates an engine for each render to isolate document metadata and checks the
native pagination trace before producing output. This adds a layout pass. Engine
pooling and performance qualification remain future work. The check catches forced
unsplittable overflow; it is not a general overflow or accessibility validator.

`Flow` uses ordinary block flow with space between rooted children. `Stack` uses
column flex layout. `Heading` requests keeping following content with it by
default. The engine remains responsible for final pagination; components do not
estimate font metrics or pre-paginate rows in Python.

`document.compile()` returns HTML, CSS, and a source map. Generated `data-fb-id`
values use the engine's existing source-ID mechanism. Source filenames and line
numbers live in the sidecar, not in the generated PDF. Translating all native
layout diagnostics into component-level messages is still future work.

## Run the candidate

From the repository, build and install a wheel into an isolated environment:

```console
python build_backend/fullbleed_build_backend.py wheel --out target/recipe-wheel
python -m pip install --force-reinstall <wheel-path-printed-by-the-build>
python examples/component_recipes/main.py --output target/recipe-examples
```

The rendering example has no optional dependencies. The evidence tool requires
the test-only readers `pypdf`, `pypdfium2`, and `pillow`:

```console
python -m pytest tests/test_fullbleed_ui_experimental.py
python -I tools/smoke_component_recipes.py --out target/recipe-verification
```

To verify the decorated function's argument types from the installed wheel,
install the test-only `mypy` package and run:

```console
python -I tools/smoke_component_typing.py --out target/recipe-verification/typing.json
```

This checks both a valid consumer and intentional argument errors. The typing
marker applies to the experimental subpackage; it does not claim that the full
native package or every style/part name has complete static typing.

Before stabilization, this prototype still needs broader row-continuation coverage,
broader layout-diagnostic translation, static typing and completion refinement,
an evaluated rendering reuse policy, and independent author usability feedback.
The legacy authoring surface is unchanged. No language parity, PDF conformance,
or performance claim follows from these examples.
