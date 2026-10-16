# Component authoring: the next iteration

Design review, 2026-10-16. Reviewed the released surface at `e1ce0e9` and
the experimental candidate at `ecc52c6`. This document recommends changes;
it does not announce a release. The candidate uses version metadata 2.5.26,
but its experimental API is absent from the published 2.5.26 wheel.
The [detailed implementation review](component-authoring-design.md) records
the existing prototype and its validation limits.

The source follow-up now implements consistent table defaults, column positions
in diagnostics, and a complete first-PDF example. These changes remain in the
experimental branch. The baseline findings below explain why they were needed;
independent author feedback and language expansion are still pending.

## Decision

Fullbleed should offer **reusable document components that own their appearance
and print behavior**. The caller supplies content and chooses documented
customizations. Importing a component should be enough to bring its styles with
it. Authors should not need to coordinate selectors, repeat table structure,
or inspect generated HTML to make an ordinary document change.

The released API removes HTML syntax and provides useful escaping and Python
composition. It still leaves much of the HTML/CSS coordination to the author.
The experimental API removes several of those responsibilities, but it now
needs consolidation. Adding more component names would increase the learning
cost without necessarily removing work.

The next milestone should be **one coherent authoring path, demonstrated by the
same design kit in an invoice and a report**. Keep the experiment in Python
until its contracts and editing experience are settled.

## What a component should buy its caller

There are two audiences: someone designing a reusable kit, and someone assembling
a document from that kit. The kit author still makes typography and layout
decisions. The document author should not have to repeat them.

| Responsibility | Owner | Consequence for the caller |
| --- | --- | --- |
| Calculations, localization and data fetching | Application code | Ordinary functions; no special expression language or hidden I/O |
| A field's label, value, spacing and appearance | The field component | Import it and pass the label and value |
| Table headers, cells, alignment and widths | Column definitions and the table component | Add a column in one place |
| Shared brand values | An optional document theme | Change a value without searching selectors |
| Paper geometry, document title and language | The document | One authoritative setting for each |
| Fonts, assets and output profiles | Application renderer configuration | Configure once outside business components |
| Measurement, pagination and PDF emission | The existing native engine | No second layout engine in the Python layer |

The following is **working candidate syntax**, using the repository's existing
[kit](../examples/component_recipes/kit.py). `Field`, `WorkTable`, `Total` and
`studio_theme` are application code, not proposed built-ins. Run this in that
example's module context; this snippet constructs a document, while its
[runner](../examples/component_recipes/main.py) supplies the renderer and fonts.

```python
from fullbleed.ui.experimental import Document, Flow, Heading, Page, mm, pt
from kit import Field, WorkTable, Total, studio_theme


def Invoice(data):
    items = tuple(data["items"])
    return Document(
        Flow(
            Heading("Invoice " + data["number"], level=1),
            Field("Prepared for", data["customer"]),
            WorkTable(items),
            Total(items, paid=data["paid"]),
            gap=pt(16),
        ),
        title="Invoice " + data["number"],
        lang="en-US",
        page=Page("A4", margin=mm(18)),
        theme=studio_theme(),
    )
```

This example earns its abstraction because the report can reuse the same kit.
A wrapper around a one-off tree would not establish that benefit. It also does
not eliminate styling knowledge for kit authors: it puts that knowledge beside
the component that owns the design.

## A small public mental model

1. **A component is a function.** Arguments express business meaning. Children
   and ordinary named arguments, such as `footer=None`, supply replaceable
   content. Conditions and iteration remain Python. `@component` optionally adds
   identity for diagnostics and a declared customization boundary.
2. **Appearance is a value.** `style(...)` composes declarations. Primitives
   accept `style=`. `.with_style(...)` derives one returned component instance;
   `styled(...)` supplies reusable root defaults. All use the same style values.
   These are different operations on the same model, not separate styling systems.
3. **Customization is explicit.** Root styling requires one real root. Named
   parts expose selected internal appearance when needed. Replace content through
   function arguments, not style parts. A fragment contributes siblings and does
   not invent an invisible layout box.
4. **The document defines the output context.** It owns paper geometry and can
   provide theme values. A simple document should not need tokens, parts, source
   maps or a prepared-layout lifecycle before it can render.

Preserve arbitrary function signatures. Injecting `style`, `class_name` or
layout arguments into every custom function creates collisions with business
arguments and weakens typing. Root customization on the returned value avoids
that problem. [PEP 612](https://peps.python.org/pep-0612/#concatenating-keyword-parameters)
explains the limitations of adding keyword parameters to a captured signature.

Keep explicit units. `pt(12)` and `mm(18)` communicate physical intent; a saved
pair of parentheses is not worth an ambiguous measurement.

## Styling and `class_name`

Teach this distinction early:

- `style=` supplies declarations to this element.
- `styled(...)` supplies reusable defaults that travel with a component.
- `class_name="notice"` adds a literal CSS class. It loads no styles.
- `Document(styles={".notice": ...})` supplies document-wide selector rules.
- A theme supplies shared values; it does not silently override explicit styles.

No class name should resolve a Python symbol, discover a neighboring stylesheet,
or register global state. Generated classes are an implementation detail and do
not isolate components from document CSS. Keep selector-based CSS available for
existing designs without requiring it in the component-first path.

There are two stages of precedence. First, authoring composes reusable defaults
and explicit declarations. The candidate compiles defaults into class rules and
explicit values into inline declarations. Then the native CSS cascade resolves
selectors, inheritance and `!important`. Do not advertise `.with_style(...)` as
an unconditional override of arbitrary external important rules, or promise
that tag selectors always beat generated classes.

## Findings in the reviewed baseline and their follow-up

### 1. Apply the default-style contract consistently

**Observed in the installed candidate:** a styled paragraph's padding is emitted
in a generated class, but `DataTable` emits its built-in header/cell padding,
borders and alignment inline. A normal `.ledger td` rule therefore cannot replace
that padding, even though the user did not explicitly choose it. Parts can
override it, but that makes the styling route depend on the component.

The cause is concrete: [`_table.py`](../python/fullbleed/ui/experimental/_table.py)
passes both library defaults and column choices through `style=`, whereas the
[node/compiler](../python/fullbleed/ui/experimental/_tree.py) distinguish defaults
from authored values. `Column.align` also eagerly defaults to `"left"`, losing
the distinction between an omitted choice and an explicit one.

**Recommendation:** put built-in table appearance in the existing default layer.
Retain explicitly supplied column widths/alignment and part overrides as authored
choices. Represent an omitted column choice separately, as the layout primitives
already do. This is a consistency change, not another styling API. Qualify normal
selectors, explicit column choices, named parts, shorthand/longhand order and
empty tables with actual rendered controls before changing the candidate.

The follow-up uses that existing default layer for all table parts. Alignment
and row-break arguments distinguish omitted/None values from explicit choices.
Installed rendered controls cover ordinary selectors, explicit column choices,
part overrides, captions and empty states. They preserve the native preference
for moving ordinary rows intact; `row_break="auto"` does not force a partial row.

Do not silently change generated defaults to zero specificity as part of this
fix. Whether ordinary tag selectors should override component defaults is a
separate compatibility decision; the current draft deliberately uses class
specificity.

### 2. Treat table identity separately from its printed label

**Observed in the installed candidate:** two columns named `"Value"` are accepted
and produce distinct internal source IDs, but both human-readable cell paths are
`DataTable -> row 1 -> Value -> cell`. The label is also restricted to a nonempty
string. A label is doing the jobs of display content and diagnostic identity.

**Recommendation:** include the column ordinal in diagnostic paths now. Consider
an optional application key only if stable column targeting is needed. Do not
require React-style keys on every static component. Rich header content and
column-specific customization should be designed together later, preserving
semantic header associations; adding a dozen unrelated table props is premature.

The follow-up adds a `column N` scope while preserving the printed label,
native source ID and diagnostic code. Header, cell, callback and native layout
failures all carry the position. No application key is required.

### 3. Make the first working PDF the beginning of the guide

**Observed in the candidate guide:** the opening example constructs a document;
the rendering section follows styling, parts, selectors, themes and tables.
The first example therefore does not yet give a new author a complete result.

**Recommendation:** lead with a complete, copyable function-to-PDF example that
selects a bundled font explicitly. Then show three edits: change one instance,
reuse the component in another document, and change a shared design value. Move
part contracts, cascade details and layout preparation into linked reference
sections. Keep one recommended spelling for each operation; do not teach every
available escape hatch at once.

The guide now begins with an independently executable program that writes
`summary.pdf`. Its installed-package check runs the exact first code block alone,
then checks text, title and paper dimensions and retains an independent preview.

### 4. Expose print intent without promising unsupported behavior

Keep `Flow` as the recommended document-body layout and distinguish local flex
groups (`Stack`/`Row`) and grids. A name should tell an author how the content
flows across pages. Keep semantic heading levels separate from visual size.

The current candidate already offers page geometry, heading keep-with-next,
explicit breaks and table continuation checks. Repeated page furniture, mixed
page families and broader overflow reporting still need explicit contracts and
native qualification. An ordinary `Header` or `Footer` component should not
appear to promise repetition. A component wrapper cannot repair a missing native
pagination behavior.

Keep impossible-layout errors connected to authored components, rows and cells.
This can save more work than a shorter constructor call. Source provenance should
remain independent of extra HTML wrappers and should not leak local paths into
delivered PDFs.

## Implementation order and acceptance

| Order | Work | Evidence required |
| --- | --- | --- |
| 1 | Unify built-in/default versus explicit styling, starting with tables | Matching rendered controls across primitives and table parts; explicit choices still win under the documented cascade |
| 2 | Shorten the path to a first PDF and provide one complete editing exercise | Run the documented example from an installed candidate; inspect its PDF, then verify the edits without undocumented setup |
| 3 | Improve table/source identity and close bounded print contracts | Ambiguous labels remain distinguishable; empty, long and unbreakable content either renders correctly or reports an actionable limitation |
| 4 | Evaluate the same tasks with independent authors | Record successful edits, internal knowledge required and points of confusion; automated checks alone do not establish ergonomics |
| 5 | Carry settled semantics to other languages | Shared fixtures for children, units, style precedence, part boundaries, page ownership and error paths; language-appropriate syntax |

Use the same fixture and five edits for the released and proposed APIs: reuse a
field in a report, rebrand, add a calculated column, change paper size, and repair
a pagination failure. Compare the places that must change and the undocumented
knowledge required. Preserve the same content, design and output requirements;
line counts from different invoice designs are not a fair comparison.

Keep the established API and its explicit migration adapter working. Stabilize
one replacement contract before deciding namespace migration or deprecation.
For JavaScript/TypeScript, evaluate functions and JSX against these semantics;
for C#, use idiomatic builders. Shared behavior does not require shared syntax
or identical generated class names.

Defer hooks, reactive state, a global theme registry, metaclass components,
automatic prop injection, a built-in business-component catalog and a new
universal document IR. None is needed to complete this iteration. The existing
document-local compiler and native engine can support the recommended work.

## Design references and evidence limits

[React's composition guide](https://react.dev/learn/passing-props-to-a-component)
supports the useful precedent of functions, props and children.
[React-pdf's style values](https://react-pdf.org/docs/v4/styling) and
[explicit page-wrapping controls](https://react-pdf.org/docs/v4/advanced/page-wrapping)
illustrate two distinct responsibilities: reusable appearance and print behavior.
[QuestPDF's address component](https://www.questpdf.com/concepts/code-patterns/components.html)
illustrates a reusable component with meaningful application inputs. These inform
the recommendation; they are not evidence of Fullbleed parity.

The original review checks inspected installed-candidate construction and compiled
HTML/CSS, including table defaults and duplicate-label source paths. They do not
establish author productivity or conformance. Follow-up implementation checks in
[`smoke_component_styles.py`](../tools/smoke_component_styles.py),
[`smoke_component_diagnostics.py`](../tools/smoke_component_diagnostics.py) and
[`smoke_component_recipes.py`](../tools/smoke_component_recipes.py) exercise actual
PDFs and their authored error paths. The prior wheel fails the new table styling
control; the candidate passes it. Eighteen retained invoice/report and edge-case
fixtures keep identical PDF bytes. These are bounded fixtures, not a claim about
every document. Broader qualification remains in the
[implementation review](component-authoring-design.md).
