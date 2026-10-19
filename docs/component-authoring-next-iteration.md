# Component authoring: the next iteration

Design review, 2026-10-16. Reviewed the released surface at `e1ce0e9` and
the installed experimental preview at `f88cced`. This document recommends
changes; it does not announce a stable API. The
[GitHub preview](https://github.com/fullbleed-engine/fullbleed-official/releases/tag/component-preview-f88cced)
uses version metadata 2.5.26, but its experimental API is absent from the
PyPI 2.5.26 wheel.
The [detailed implementation review](component-authoring-design.md) records
the existing prototype and its validation limits.

The source branch now prepares the separately versioned `2.6.0a1` alpha. Its
[runbook](release/2.6.0a1-runbook.md) defines installation, publication gates and
the corresponding Cargo version. The older GitHub downloads and the observations
against them below remain pinned to their original commits.

The next implementation now uses [Bootstrap 5.0.0 as the baseline kit](component-bootstrap.md),
following the requested scope. It adds owned stylesheet dependencies, static
component families, content slots, generated group IDs, checked utilities, and
HTML export with or without styles. Explicit print representations handle
browser controls and navigation without adding a browser to the engine. This
addresses everyday composition. The source branch now implements the explicit
part-binding proposal below; independent-author feedback remains open. Other language APIs should follow the settled
content/style/print contract, rather than copying Python syntax prematurely.

The preview already implements component-owned styles, immutable overrides,
data tables, page ownership and component diagnostics. Table defaults and the
first-PDF guide have also been improved. The next problem is making those
mechanisms form a small, dependable authoring contract. Independent author
feedback and language expansion are still pending.

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

Evaluate a change by the decisions it removes from the caller. A reusable
component should survive a change to its internal markup, move into another
document with its appearance intact, and explain a failed layout in the author's
terms. A shorter spelling of the same HTML tree is useful syntax, but does not
establish those benefits. Low-level primitives can remain explicit; the common
document workflow should operate through meaningful components built from them.

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

The following is **working preview syntax**, using the repository's existing
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

For the common path, teach function arguments for content and variants,
`.with_style(...)` for one root, and `styled(...)` when defaults repeat. Introduce
themes and parts only when a concrete reuse task needs them. A caller changing
the content of a footer should pass a `footer=` argument; a style part should
not become a second component-composition language.

There are two stages of precedence. First, authoring composes reusable defaults
and explicit declarations. The candidate compiles defaults into class rules and
explicit values into inline declarations. Then the native CSS cascade resolves
selectors, inheritance and `!important`. Do not advertise `.with_style(...)` as
an unconditional override of arbitrary external important rules, or promise
that tag selectors always beat generated classes.

## Contracts the preview has not settled

### 1. A public part needs an explicit target

**Observed in the published `f88cced` and `41a2cee` previews:** a component that directly returns another
component delegates its parts. Adding a real container makes the child a nested
boundary and hides those parts. Declaring the same names on the parent makes
them available again, but does not forward their styles into that boundary.

This small example reproduces the problem in those published previews. The
updated source branch instead rejects `Summary("Ready")` at construction:

```python
from fullbleed.ui.experimental import Box, Document, Paragraph, component, style

@component(parts=("label",))
def Label(text):
    return Paragraph(text, part="label").with_style(color="#315d9b")

@component(parts=("label",))
def Summary(text):
    return Box(Label(text))

before = Summary("Ready")
after = before.with_parts(label=style(color="#245541"))
assert before.parts == ("label",)
assert Document(before).compile() == Document(after).compile()
```

The override succeeds without changing the output. Allowing an absent declared
part is intentional for conditional and empty states; it also conceals missing
forwarding. A list of names establishes spelling, not a binding contract.

**Implemented in the source prototype:** `parts=("label",)` requires a real
target; `optional_parts=("note",)` permits a conditional target to be absent.
To fix the example, return `Box(Label(text).forward_parts("label"))`.
Aliases such as `.forward_parts(heading="label")` expose a child part using
the parent's vocabulary. A required target missing at construction raises
`component.part_missing`; merely repeating a child's part name no longer
pretends to connect it. The public preview downloads remain unchanged.

[`_tree.py`](../python/fullbleed/ui/experimental/_tree.py) resolves bindings at
component construction. A binding reaches an owned marker or a deliberately
forwarded child part, never a search through private descendants. Ordered tree
paths identify occurrences, so aliases and repeated table cells keep their
order while shared child instances remain independently customizable. Zero
targets are valid for explicitly optional parts. All forwarding is metadata;
it adds no elements or layout boxes, and does not run component functions again.

[Focused tests](../tests/test_fullbleed_ui_parts.py) cover these boundaries,
including nested aliases, optional-only fragments, multi-level forwarding,
deep trees, shared nodes and style composition. The
[installed-wheel check](../tools/smoke_component_parts.py) compares rendered
controls for the same override before and after inserting a container, private
labels, shared children, empty and paginated tables, and a Bootstrap card.
It also checks styled/unstyled HTML. This is a candidate API change awaiting
author feedback, not a stable compatibility promise. Ordinary content arguments
remain the first choice for common customizations.

Local Windows qualification on 2026-10-16 used a rebuilt, installed wheel
(SHA-256 `30fe308dd7d4d1b9daf37dfff2b0672b68159003a53a52387a5c0f32b2317437`).
All seven rendered cases, spanning eleven pages, match their controls in PDFium
pixels and retain repeatable PDF bytes. The existing Bootstrap catalog, report
and long table retain all 29 PDF byte sequences and five HTML exports from the
public `41a2cee` wheel. Mypy rejects 55 deliberate errors; Pyright rejects 56
and passes 24 actual completion requests. Dynamic part names still need runtime
validation. All 893 Python tests pass with the installed native extension,
including 33 focused part-binding cases. Raw evidence lives under
`target/growth/component-parts-20261016/`;
CI runs the same new parts check and retains its PDFs, previews and report under
`component-recipes-<OS>/parts/`. These checks do not establish usability or wider
CSS compatibility.

### 2. Defaults and decisions need a kit-author convention

**Observed in the installed preview:** wrapping a function with
`styled(Function, color=...)` cannot replace a color that the function explicitly
sets with `style=` or `.with_style(...)`. Applying `.with_style(...)` to the
returned node can replace its root color. Neither operation automatically
restyles a descendant with its own explicit color.

These are consistent rules, but the difference is easy to lose when every
example calls all declarations "styles." A kit author needs to know whether a
value is a reusable default, a deliberate component decision or an instance
customization.

**Recommendation:** keep one immutable style value and document those three
roles together. Put reusable appearance on styled primitives; use function
arguments for meaningful choices such as `paid` or `tone`; use instance overrides
for local changes. Avoid a second `sx` prop, automatic keyword injection or a
variant registry. Keep the native cascade explicit, including inheritance and
important declarations. An internal style is not necessarily a public theming
point just because it can be expressed in CSS.

The preview's table-default inconsistency is already fixed: built-in appearance
uses the default layer, while explicit column choices remain authored styles.
Preserve that distinction across components. Changing generated defaults from
class specificity to zero specificity would require a separate compatibility
decision and rendered qualification.

### 3. Importing a component can still import a hidden theme requirement

**Observed in the installed preview:** a component using `token("accent")`
cannot compile without a document binding for that name. Two independent kits
using `accent` share the same document value. `required_tokens` makes that
dependency inspectable, but does not provide defaults, semantic types or scopes.

**Recommendation:** starter components should work with concrete defaults.
When a kit needs a configurable palette, an ordinary immutable application
configuration passed to a kit factory is sufficient. If a kit opts into document
tokens, ship its complete theme and document the dependency. Use explicit kit
prefixes such as `invoice.accent` when independent meanings must coexist; the
preview already accepts these names. Do not describe a component as completely
self-contained when it requires an unspecified theme.

Evaluate typed token handles or theme scopes only after composing two independent
kits demonstrates the missing contract. A global theme registry would introduce
implicit state without solving ownership. Likewise, copying a kit into another
document should not require unpacking its implementation to discover required
values.

### 4. Component execution is eager; layout is a later phase

**Observed in the installed preview:** `DataTable` consumes rows and calls cell
functions during construction. Compiling the resulting document again does not
rerun those callbacks. Component functions likewise run as ordinary Python calls.

**Recommendation:** make this timing part of the contract. Application code owns
I/O and calculations; construction captures their results; compilation resolves
styles; the native engine lays out the document. Immutable nodes do not make an
impure function deterministic. A table is currently a captured tree, not a
streaming interface. Future page-number or page-dependent content needs a
separately specified native mechanism, not hidden re-execution of user functions.

### 5. Print behavior must remove decisions, too

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

The preview already distinguishes duplicate column labels by position and starts
its guide with a complete `summary.pdf` program. Those are useful improvements.
The next print work should close bounded behaviors such as oversized rows and
repeated page furniture, with actionable errors for unsupported cases. Adding
another generic container name would not close those gaps. `row_break="auto"`
permits splitting; it does not force an otherwise fitting row to split.

The [page-furniture implementation design](component-page-furniture-design.md)
now records the next authoring contract and a reproduced native prerequisite:
generated page numbers are unmarked in the public stable and alpha engines.
Its candidate correction and saved-PDF regression gate precede exposing new
constructors. Rich-slot sizing, fit diagnostics, record numbering and HTML
export remain explicit acceptance requirements for that authoring work.

## Implementation order and acceptance

| Order | Work | Evidence required |
| --- | --- | --- |
| 1 | Public part binding/forwarding (implemented in the source prototype) | A wrapper refactor preserves advertised customization; nested private parts remain private; an absent optional target is distinguishable from broken wiring; independent author feedback is still needed |
| 2 | Consolidate the recommended authoring path and one default kit | First PDF, reuse, content replacement and local restyling need no selectors or token/part machinery; advanced recipes explain their dependencies |
| 3 | Close bounded print contracts in the native engine | Empty, long and unbreakable content renders correctly or reports an actionable limitation; repeated page elements have explicit semantics |
| 4 | Evaluate the same tasks with independent authors and two independent kits | Record successful edits, internal knowledge and integration failures; use that evidence to decide token/part typing and discovery work |
| 5 | Carry settled semantics to other languages | Shared fixtures for children, execution timing, units, style precedence, part boundaries, page ownership and error paths; language-appropriate syntax |

Use the same fixture and five edits for the released and proposed APIs: reuse a
field in a report, rebrand, add a calculated column, change paper size, and repair
a pagination failure. Compare the places that must change and the undocumented
knowledge required. Preserve the same content, design and output requirements;
line counts from different invoice designs are not a fair comparison.

Also refactor the internals of one customized component without editing its
callers, and compose two kits with different palettes. These expose whether the
API has actually established reusable boundaries. Record edits to component
code, selectors, document configuration and renderer configuration separately.
Moving boilerplate into a hidden helper is not the same as eliminating it.

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

This pass also repeated eight construction/compilation probes against the
installed `f88cced` preview, checking its four relevant Python modules against that
commit. They cover default versus explicit styling, part delegation and the
accepted no-op above, theme dependencies and namespaced values, and eager table
callbacks. They establish those API behaviors; they add no new rendering,
performance or usability claim. The source follow-up now qualifies explicit
bindings with the rendered controls described in section 1. The observed
behavior remains documented here so the change from the pinned downloads is
reviewable.
