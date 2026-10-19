# Experimental component authoring

This prototype explores a document API with reusable styling, named component
parts, theme values, page geometry, and data tables. It is available from this
source branch's wheel under `fullbleed.ui.experimental`; it is not in the
PyPI 2.5.26 wheel. Names and contracts may change before stabilization.

The `2.6.0a8` alpha has its own package version. Install it explicitly with
`python -m pip install --pre "fullbleed==2.6.0a8"` in a virtual environment, or
[build an unpublished checkout](#run-the-candidate). The
[alpha runbook](../../docs/release/2.6.0a8-runbook.md) records the release gates.

The [implementation review](../../docs/component-authoring-design.md) explains
the design decisions, remaining gaps, and criteria for stabilizing this API.

The [invoice and report](documents.py) use the same [design kit](kit.py). Neither
document imports a stylesheet or coordinates selectors. The kit contains the
styles and the components that use them. The example data is the existing
fictional [Northstar invoice data](../components/data.json).

## Make your first PDF

A component takes ordinary arguments and returns a tree. After installing the
[candidate](#run-the-candidate), save this as `summary.py` and run
`python summary.py`. It writes `summary.pdf` using an explicitly selected bundled
font. Themes and named parts are optional; this example needs neither:

```python
from importlib.resources import files
from pathlib import Path
from fullbleed.ui.experimental import (
    Document, Flow, Heading, Page, Paragraph, Renderer, Stack, component, mm, pt,
)

@component
def Field(label: str, value: str):
    return Stack(
        Paragraph(label).with_style(font_size=pt(8), color="#667580"),
        Paragraph(value),
        gap=pt(4),
    )

body = Flow(
    Heading("Project summary", level=1),
    Field("Prepared for", "Maple & Finch"),
    Field("Status", "Ready for review"),
    gap=pt(14),
).with_style(font_family="Inter", font_size=pt(11), color="#263b46")
document = Document(body, title="Project summary", page=Page("A5", margin=mm(16)))
font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
renderer = Renderer(font_files=[str(font)])
Path("summary.pdf").write_bytes(document.render_pdf(renderer))
```

The label's appearance travels with `Field`; the document does not attach a
stylesheet. Change the label color in `Field` to update both fields, or use
`Field(...).with_style(background="#f2f5f8", padding=pt(8))` to customize one
instance. Reuse the same function in another document without stylesheet wiring.
`@component` adds a diagnostic boundary; ordinary undecorated functions can also
compose nodes. Renderer and font setup belong at the application boundary.

## Reuse styles and expose customization

When labels repeat across components, give their shared appearance a name.
When consumers need to restyle an internal label, expose it as a public part.
This version of the same field adds those two contracts:

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

`styled` captures reusable root defaults. Compilation collects those defaults
and generates class names from the resolved declarations. Explicit layout
arguments, `style=`, and instance overrides become inline declarations. There is
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

Declare required targets with `@component(parts=(...))` and conditional targets
with `optional_parts=(...)`. A required part must reach at least one element;
otherwise construction raises `component.part_missing` with the component and
part name. An optional note or an empty table's cells can receive overrides while
absent, without making callers branch on the content. These explicit required /
optional declarations are an alpha update; the published
`component-preview-41a2cee` still treats all declared parts as optional.
Authored `part=` markers must match the declaration; misspellings fail when the
component is constructed. Nested components validate their own declarations.
An explicit declaration on a delegating wrapper can expose a subset of its root's
parts. An undeclared wrapper retains root delegation.

To expose a nested child's part, forward it deliberately:

```python
from fullbleed.ui.experimental import Box, Paragraph, component, style

@component(parts=("label",), optional_parts=("note",))
def Notice(text, note=None):
    return Box(
        Paragraph(text, part="label"),
        Paragraph(note, part="note") if note is not None else None,
    )

@component(parts=("heading",), optional_parts=("details",))
def Summary(text, note=None):
    return Box(Notice(text, note).forward_parts(heading="label", details="note"))

summary = Summary("Ready").with_parts(heading=style(color="#315d9b"))
```

`forward_parts(heading="label")` exposes the child's `label` under the parent's
name `heading`. `forward_parts("label")` keeps the name. It adds no HTML element,
does not run the child again, and survives inserting ordinary containers around
it. Other child parts remain private. Only a child's advertised parts can be
forwarded; repeating the same public name on several children targets each
occurrence in order. A reused child can be customized in one occurrence without
changing another. Content still comes through ordinary function arguments.

Forwarding returns a new node and leaves that node's own `.parts` unchanged;
the selection applies when a parent consumes it. A second `forward_parts` call
replaces the selection. Explicit forwarding also replaces implicit delegation
when returning a child directly. An optional child target cannot satisfy a
parent's required part while absent. When multiple aliases target the same
element, `.with_parts(...)` composes their styles in keyword order.

`Field.parts` exposes its declaration, and `field.parts` exposes the instance's
public names. Plain `@component` remains supported and discovers names from
present content; use an explicit declaration for a stable customization contract.
`ComponentFactory` preserves a decorated function's argument types while exposing
its declaration. `.parts` lists required then optional names; `.optional_parts`
lists the declared optional names. Factories with inferred parts expose `None`
for both declarations. Styled factories preserve both; styling a primitive
exposes `parts=None` and `optional_parts=None`.

Styles compose in this order: primitive defaults, styled defaults from inner to
outer, explicit layout arguments, `style=` declarations, then later `with_style`
or `with_parts` changes. Overrides move to the end, preserving
CSS shorthand/longhand ordering. Themes supply values and document text defaults;
they do not silently override a component's own settings.

`styled` forwards the wrapped function's arguments unchanged. A primitive such
as `Paragraph` accepts `style=` declarations; a custom function decides what its
own arguments mean. A recipe supplies defaults without overriding explicit styles
inside that function. Use `.with_style(...)` on its result to override them:

```python
@component
def Invitation(*, style: str):
    return Paragraph("Invitation: ", style)

StyledInvitation = styled(Invitation, font_size=pt(12))
invitation = StyledInvitation(style="formal").with_style(color="#315d9b")
```

A custom function without a `style` argument does not gain one through `styled`.
Earlier prototype callers using that shortcut should move the declarations to
`.with_style(...)` after the call. See the
[implementation review](../../docs/component-authoring-design.md#composition-contracts-to-settle-before-adding-more-api)
for the fragment, spacing and argument-forwarding contracts.

Layout arguments follow the same ownership rule. Omit an argument, or pass
`None`, to use a reusable default. Supplying a value makes an explicit choice,
including zero spacing and `keep_with_next=False`:

```python
from fullbleed.ui.experimental import Flow, Paragraph, pt, styled

Spaced = styled(Flow, row_gap=pt(12))
default_spacing = Spaced(Paragraph("One"), Paragraph("Two"))            # 12 pt
inherited_spacing = Spaced(Paragraph("One"), Paragraph("Two"), gap=None)  # 12 pt
compact_spacing = Spaced(Paragraph("One"), Paragraph("Two"), gap=pt(3))   # 3 pt
touching = Spaced(Paragraph("One"), Paragraph("Two"), gap=0)               # 0
```

`style=` can override an explicit layout argument, and `.with_style(...)` can
override the result. The rule also applies when a custom function forwards an
optional argument, such as `gap=None`, to a primitive. A function that explicitly
chooses a gap keeps that choice when another module adds reusable styling.

The built-in visual defaults remain: 8 pt spacing; stretch alignment for `Stack`;
center alignment and space-between justification for `Row`; two equal columns for
`Grid`; and keeping `Heading` with following content. `None` is the public default
for those optional layout arguments so reusable defaults can take effect.

`class_name` still means a literal CSS class label. It does not load styles or
refer to a Python variable. In this prototype, use style values for the normal
authoring path. `css("transform: rotate(2deg)")` is an explicit declaration escape
hatch for CSS beyond the checked property vocabulary. To use selectors, attach
their rules explicitly with `Document(styles=...)`.

Choose the control that matches the change:

| Change | Use |
| --- | --- |
| Business meaning, such as a paid invoice | An ordinary function argument, such as `Total(items, paid=True)` in the example kit |
| Appearance of one component root | `field.with_style(background="#f2f5f8")` |
| Defaults reused by many elements | A `styled(...)` factory, such as `Label` above |
| A public internal label or cell | `.with_parts(label=style(...))`, using the component's advertised parts |
| Shared brand values or document typography | `Theme` tokens and `Theme.text` |
| Existing selector-based CSS | `class_name` with `Document(styles=...)`; use the [adapter](#adopt-components-in-an-existing-document) when mixing APIs |

A `class_name` alone supplies a label, not an appearance. Do not pass a
`StyleSheet` or a style value as a class name.

## Use existing CSS with class names

`styles` accepts CSS text, a selector mapping, or the established
`fullbleed.ui.StyleSheet`. The document snapshots that input; later edits to a
shared sheet or mapping do not change an existing document.
Other stylesheet objects can implement `to_css() -> str`; it is called once
during document construction and its returned text goes through the same checks.
Selector mappings also accept the same `style(...)` values used on components,
including theme tokens. Compilation resolves those tokens and checks stylesheet
syntax and page ownership before native layout.

```python
from fullbleed.ui.experimental import Document, Page, Paragraph, mm, pt, styled

Notice = styled(Paragraph, font_size=pt(18), color="#a0442e")
notice = Notice("Paid in full", class_name="notice")
document = Document(
    notice,
    page=Page("A5", margin=mm(15)),
    styles={".notice": {"color": "#315d9b"}},
)
```

The notice prints blue. For a themed rule, use
`styles={".notice": style(color=token("accent"))}` and supply that token through
the document's theme. `document.required_tokens` includes tokens in these rules.

Generated default rules precede the document stylesheet;
ordinary specificity and source order apply. A selector such as `p` has lower
specificity than a generated class, while `.notice` has equal specificity and
wins by appearing later. Class order in the attribute has no effect.

An explicit `notice.with_style(color="#245541")` prints green with those same
ordinary class rules. Explicit styles use inline precedence; an external
`!important` rule can still override a normal inline declaration. Theme text
styles supply body defaults and participate in ordinary inheritance. There is
no Python implementation of the CSS cascade.

`Page` owns paper size, orientation and page margins. The native CSS parser
rejects competing `@page` geometry in `styles`, including named, pseudo-page,
conditional and layered rules. This ownership check covers inactive conditions
too, so changing context cannot activate a second paper configuration. Ordinary
element margins and page furniture such as `@bottom-center` remain available.
Add selectors through `styles`; embedding a `Node("style", ...)` is rejected.
Advanced documents needing CSS-controlled page families can continue using the
established HTML/CSS engine interface.

Style fragments support normal Python conditionals:

```python
from fullbleed.ui.experimental import style

paid = False
appearance = style(color="#233c36")
appearance = style(appearance, paid and style(font_weight=700), margin=0)
```

`None` and `False` fragments are omitted. Zero-valued declarations remain zero;
`True` is not a style fragment.

## Rebrand a shared theme

Derive a theme with `with_values(...)`; its text style travels with it. The
original theme and documents using it remain unchanged:

```python
from fullbleed.ui.experimental import Document, Paragraph, Theme, style, token

note = Paragraph("A portable note", style=style(color=token("accent")))
base_theme = Theme({"accent": "#315d9b", "ink": "#263b46"},
                   text=style(font_family="Inter", color=token("ink")))
warm_theme = base_theme.with_values(accent="#9a462e")
warm_document = Document(note, theme=warm_theme)

assert note.required_tokens == ("accent",)
assert warm_document.required_tokens == ("accent", "ink")
```

`with_values` also accepts mappings, such as `base_theme.with_values(brand_values)`.
Later mappings win, then keyword values win. Updates are captured immediately
and use the same name/value validation as theme construction. Update or add token
values here; create a `Theme(..., text=...)` when changing the text-style definition.

`required_tokens` lists sorted, unique explicit `token(...)` references in the
constructed component or document. It crosses nested component boundaries, uses
the final style layers, and includes document-wide text and selector styles. An overridden
default or absent optional part adds no requirement. Extra values supplied by a
theme are not requirements. Discovery does not resolve a theme or rerun component
functions; missing values still report their component path during compilation.

Requirements depend on actual arguments and content, so inspect the returned node,
not the component factory. CSS `var(...)` references are a separate CSS mechanism
and are not inferred as theme tokens. This property exposes the names at runtime;
automatic completion of arbitrary kit-specific token names remains future work.

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

Table padding, borders, caption spacing and default alignment use the same
default-style layer as other components. Normal selector rules can customize
them; named-part overrides are explicit instance choices. `Column(align=None)`
or an omitted alignment allows the left-aligned default or stylesheet rule to
apply. An explicit `align="left"`, `"right"`, or `"center"` and an explicit width
take inline precedence on that column's header and cells. Likewise, omitted/None
`row_break` allows the default `avoid` preference or selector styling; an explicit
`row_break="avoid"` or `"auto"` takes inline precedence. Native `!important`
rules still apply.

Table parts are `header`, `cell`, `row`, `caption`, and `empty`, whether or not the
current data produces cells, a caption, or an empty-state row. Each body cell retains its table,
row, column position and printed label in the compiled source map. Duplicate
labels remain distinguishable, for example `row 1 → column 2 → Value → cell`.

**Pagination:** ordinary rows move intact when they fit on a fresh page.
This remains the native preference with `row_break="auto"`; removing an avoidance
request does not force a split at the remaining page space. This distinction
follows the difference between allowed and forced breaks in the
[CSS fragmentation model](https://www.w3.org/TR/css-break-3/#break-within).
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
from fullbleed.ui.experimental import Page, Renderer, Theme, mm, style

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
It creates an isolated engine, prepares one layout, and checks that layout's
native pagination trace before producing output. The check catches forced
unsplittable overflow; it is not a general overflow or accessibility validator.

When you need a PDF and previews, prepare once:

```python
prepared = document.prepare(renderer)
Path("customer.pdf").write_bytes(prepared.render_pdf())
preview_pages = prepared.render_image_pages(dpi=96)
pagination = prepared.export_render_time_pagination_trace()
```

The result is the engine's existing `fullbleed.CompiledDocument`. It owns its
layout, fonts and delivery options and survives the temporary engine. These
operations reuse that layout; changing content, styles or renderer settings
requires a new preparation. Calling the two `document.render_*` conveniences
separately prepares twice. Layout PNGs preview native drawing commands, not the
final PDF's encoding or conformance. Engine pooling and broad performance
qualification remain future work.

Choose `Flow` for a flowing document body and `Stack` for a local flex group such
as the field above. `Flow` uses native paginated block layout with explicit space
between children; `Stack` uses column flex layout. They have different pagination
behavior. `Heading` requests keeping following content with it by default. The
engine remains responsible for final pagination; components do not estimate font
metrics or pre-paginate rows in Python.

`Flow` owns spacing independently of its children. The preceding child's bottom
margin, the gap, and the following child's top margin add; child margins do not
collapse into each other or into the `Flow` container. Moving a shared child
between flows does not change its styles. There is no gap before the first item
or after the last item on a page; a page break discards the inter-item gap.
Authored margins and padding remain separate from that rule.

A component returning `Fragment(...)` contributes siblings in any position.
Its source identity remains available for diagnostics without adding an HTML
wrapper. Empty fragments add no box or gap. Wrap direct text in `Paragraph`.
Prefer physical gaps such as `pt(8)` or `mm(3)`. Font-relative gaps use the flow's
font size; percentage gaps need a definite container height and otherwise resolve
to zero.

This spacing uses Fullbleed's experimental native `data-fb-flow` annotation and
`row-gap` declaration. A browser does not apply gaps to ordinary block layout,
so use the generated PDF or Fullbleed preview to judge it. Unmarked HTML retains
the engine's ordinary margin-collapse behavior. The legacy adapter preserves
the annotation when embedding a new `Flow` in an existing Fullbleed document.

`document.compile()` returns a `DocumentSource` containing HTML, CSS, and a source
map. It performs no native layout or font loading. Earlier prototype code that
imported `CompiledDocument` from `fullbleed.ui.experimental` should import
`DocumentSource` instead. The native `fullbleed.CompiledDocument` returned by
`prepare` keeps its existing name and behavior. Generated `data-fb-id` values
use the engine's source-ID mechanism. Source filenames and line numbers identify
component definitions in the sidecar, not in the generated PDF.

Compilation checks paragraph/inline content, table and list parent-child
boundaries, and nested links before serialization. Fragments and component
boundaries stay transparent to these checks. For example, a paragraph containing
another paragraph raises `AuthoringError` with code `structure.child` and the
authored component path. Place those paragraphs beside each other in a `Flow`.
These checks cover the component content boundaries, not every HTML constraint.
Inline SVG descendants stay under the engine's SVG handling.

Request the established accessibility semantics checks explicitly:

```python
source = document.compile(a11y_mode="raise")
report = source.a11y_report
assert report is not None and report["ok"]
prepared = document.prepare(renderer, a11y_mode="raise")
Path("customer.pdf").write_bytes(prepared.render_pdf())
```

`a11y_mode="raise"` stops on semantic errors before creating an engine or writing
artifacts. `"warn"` emits `A11yWarning`; `None` omits the optional semantic checks.
The argument is available on `compile`, `to_html`, `to_css`, `prepare`, both
`render_*` methods, and `emit_artifacts`. Structural content checks always run.
The two calls above demonstrate source inspection and native preparation; use
just `prepare` when source inspection is unnecessary.

The existing `A11yContract` supplies the rules and `A11yValidationError` exception.
Its report remains available as `error.report`; `DocumentSource.a11y_report`
returns an independent report value when checks were requested. Diagnostics keep
their existing codes and DOM paths and add `source` with the component path,
definition file and line when available. Duplicate IDs also identify the first
occurrence in `related_sources`. The installed smoke checks that valid output
matches unchecked output; these authoring checks do not establish PDF/UA
conformance or replace delivery-profile verification.

An unsplittable native fragment raises `LayoutError`, a subclass of
`AuthoringError`. Its `page` and `sources` retain the page and affected authored
component roots. Its `reason` and `failure_sources` report the constraint rejected
by the native splitter when it can identify one. For example, an oversized,
bottom-aligned cell reports `table_cell_alignment` with a path such as
`WorkTable → DataTable → row 2 → column 1 → Description → cell` and advice to use top alignment.
A cell whose content cannot fit or continue reports `table_cell_content`.

These details come from the actual split attempt. A completed companion cell is
not blamed for a later continuation failure. A container that clips overflow is
identified as the constraint; the renderer does not blame its child text.
An oversized header or parent padding can instead identify the owning table or
container. `error.to_dict()` includes both the original scope and these failure
sources, with component-definition files and lines. They are not call-site locations.

Multiple sources remain separate; the message does not arbitrarily select one.
Some layout wrappers still provide only their scope, and an unknown native ID is
not guessed from a selector. Other native diagnostics, general overflow detection,
and precise causes through every layout model remain future work. The installed
[diagnostic examples](../../tools/smoke_component_diagnostics.py) check six failures
and render corrected documents after changing alignment, clipping and padding.

## Adopt components in an existing document

Use `to_legacy` at the boundary between a new component and an existing
`fullbleed.ui` document. The component keeps its styles and part overrides;
the existing application keeps its stylesheet and rendering configuration:

```python
from fullbleed.ui import (
    Document as LegacyDocument, Paragraph as LegacyParagraph, StyleSheet,
)
from fullbleed.ui.experimental import to_legacy

migration_theme = Theme({"muted": "#667580"})

@LegacyDocument(
    title="Existing report",
    styles=StyleSheet({".legacy-note": {"color": "#667580"}}),
)
def existing_report():
    return (
        LegacyParagraph("This section keeps its existing stylesheet.", class_name="legacy-note"),
        to_legacy(field.with_parts(label=style(font_weight=700)), theme=migration_theme),
    )

mixed = existing_report()
assert "Maple &amp; Finch" in mixed.to_html()
```

Keep using the existing document's `PdfEngine` and rendering call. The adapter
returns a tuple of legacy elements/text accepted as children, including multiple
roots from a fragment. It adds no wrapper and copies no HTML strings. Each call
creates an independent snapshot; mutating the legacy elements cannot change the
original component. Passing a new node directly into the old API raises an error
pointing to the adapter instead of printing the node's representation.

Component styles become inline declarations. `class_name` remains available to
the existing stylesheet, and inline declarations participate in that document's
CSS cascade. This bridge does not isolate document-wide selectors. Tokens resolve
from `theme`; `Theme.text` still belongs to the document. If adopting its text
defaults, add `StyleSheet({"body": theme.text.resolve(theme)})` to the existing
document once. Font selection, page geometry and PDF metadata remain its choices.

This is an incremental bridge into `Element`/`DocumentArtifact`. It does not
convert an entire old document to the experimental `Document`, load external
stylesheets, or add experimental renderer checks/source maps to the old renderer.
Once the component tree has migrated, switch the outer document and explicitly
review its page, theme and renderer settings. The migration smoke below compares
mixed documents with equivalent experimental documents, including repeated table
headers and fragments.

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
python -I tools/smoke_component_preparation.py --out target/recipe-verification/preparation
python -I tools/smoke_component_semantics.py --out target/recipe-verification/semantics
python -I tools/smoke_component_themes.py --out target/recipe-verification/themes
python -I tools/smoke_component_migration.py --out target/recipe-verification/migration
```

The wheel bundles generated type stubs for style keywords, primitive props,
layout choices, document options, table record types, and component arguments.
Editors using Pyright can suggest `padding`, `font_size`, `class_name`, table
`columns`, and valid alignment values. A misspelled `style(paddding=pt(8))` is
reported before rendering. The property vocabulary and layout choices come from
the same definitions used by runtime validation.

To verify the installed types, install the test-only `mypy` package and run:

```console
python -I tools/smoke_component_typing.py --out target/recipe-verification/typing.json
```

This executes a valid consumer and checks deliberate errors using strict typing
with Python 3.10 as the target. The independent editor check uses the official
Pyright CLI and language server. Node and Pyright are test-only tools:

```console
npm install --ignore-scripts --no-package-lock --no-save --no-audit --no-fund --prefix target/component-editor-tools pyright@1.1.414
python -I tools/smoke_component_editor.py --pyright-root target/component-editor-tools/node_modules/pyright --out target/recipe-verification/editor.json
python tools/generate_component_stubs.py --check
```

Runtime checks still own finite values, units, CSS syntax, token names and part
names. For example, `padding=(pt(4), 8)` needs a unit on its second value, including
when that value comes from a theme token. Recursive iterable child types are
checked differently by type checkers; `Flow` always rejects unwrapped text at
runtime. For full argument checking on a styled custom function, call it normally
and apply `.with_style(...)` to the resulting node. `styled` preserves the
wrapped function's signature: it does not add a `style=` argument. If that
function already accepts `style`, the argument keeps the function's original
meaning and type.

The typing marker applies to the experimental subpackage. Automatic token/part
name completion and broader native-package typing remain separate work.

Before stabilization, this prototype still needs broader row-continuation coverage,
broader layout-diagnostic translation, static typing and completion refinement,
larger-scale rendering qualification, and independent author usability feedback.
The legacy authoring surface is unchanged. No language parity, PDF conformance,
or performance claim follows from these examples.
