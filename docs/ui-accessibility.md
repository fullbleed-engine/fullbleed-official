# `fullbleed.ui` Accessibility Authoring

This guide covers the component-first HTML authoring helpers in `fullbleed.ui`,
with a focus on `fullbleed.ui.accessibility` for remediation-oriented document
workflows.

## Modules

- `fullbleed.ui`: core HTML/component authoring (`Element`, `Document`, `to_html`)
- `fullbleed.ui.primitives`: engine-safe layout and presentation primitives
- `fullbleed.ui.style`: inline style composition (`Style`, `style(...)`)
- `fullbleed.ui.accessibility`: semantic wrappers and a11y validation

Scaffold starter:

- `fullbleed new accessible <path>` creates an accessibility-first local project scaffold using these modules.
- The scaffold renders through `fullbleed.accessibility.AccessibilityEngine` and emits verifier/PMR/PDF seed artifacts plus non-visual traces by default.

## Runtime Surface (`fullbleed.accessibility`)

Authoring and runtime/output are intentionally separated:

- `fullbleed.ui.accessibility`: semantic authoring primitives + `A11yContract`
- `fullbleed.accessibility`: PDF/UA-targeted runtime wrapper (`AccessibilityEngine`)

Use the UI layer to author semantic HTML. Use the accessibility runtime surface
to emit artifacts and audit/trace outputs for review and CI.

## Core Pattern

Use `@Document(...)` to compose a document artifact, then emit HTML with
`artifact.to_html(...)`.

```python
from fullbleed.ui.core import Document
from fullbleed.ui import el
from fullbleed.ui.accessibility import FieldGrid, FieldItem

@Document(title="Example", bootstrap=False)
def App():
    return el("div", FieldGrid(FieldItem("Name", "Jane Doe")))

artifact = App()
html = artifact.to_html(a11y_mode="raise")
```

`a11y_mode` values (v1):

- `None`: no automatic validation during HTML emission
- `"warn"`: emit diagnostics as warnings and return HTML
- `"raise"`: raise `A11yValidationError` on structural errors

## Inline Styles (`fullbleed.ui.style`)

`style=` accepts strings and composed values.

```python
from fullbleed.ui import el, style

node = el(
    "div",
    "hello",
    style=style({"font_weight": 700}, "color: #123;", {"margin_top": "4px"}),
)
```

Behavior:

- preserves authored/insertion order
- normalizes `snake_case` properties to kebab-case
- warns on suspicious values/types (for example raw booleans)

## Accessibility Module Highlights

### Semantic tables

Use `SemanticTable*` wrappers for data tables. This keeps table semantics
distinct from generic layout primitives.

```python
from fullbleed.ui.accessibility import (
    SemanticTable, SemanticTableHead, SemanticTableBody, SemanticTableRow,
    ColumnHeader, RowHeader, DataCell
)
```

### Field/value semantics

Use `FieldGrid` + `FieldItem` for non-tabular label/value content.

- `FieldGrid` is semantic-first and emits `dl/dt/dd`
- use `LayoutGrid` (in `fullbleed.ui.primitives`) for non-semantic box layout

### Landmarks, sections, status

Available wrappers include:

- `Region`, `Heading`, `Section`
- `Status`, `Alert`, `LiveRegion`
- `FieldSet`, `Legend`, `Label`, `HelpText`, `ErrorText`
- `Details`, `Summary`
- `SrText`

## `A11yContract` Validation

`A11yContract` is a lightweight structural validator intended for document
authoring and remediation workflows.

Current checks include:

- duplicate IDs
- missing `aria-labelledby` / `aria-describedby` targets
- empty `aria-label`
- multiple `main` landmarks
- unlabeled `region`
- informative image text alternatives
- signature enum validation (`signature_status`, `signature_method`)

Example:

```python
from fullbleed.ui.accessibility import A11yContract

report = A11yContract().validate(artifact, mode="warn")
```

## Signature Semantics (Text First)

Model signatures as two separate concerns:

1. Meaningful signed-state content (textual/machine-readable)
2. Visual signature mark (supplemental, optionally decorative)

Use:

- `SignatureStatus`
- `SignatureMark`
- `SignatureBlock`

Guidance:

- do not rely on a signature image alone to convey signed state
- when a signature image carries meaning, provide text equivalent including
  `Signature` and the signer name (for example `Signature: Jane Doe`)
- if the mark is redundant, make it decorative (`Decorative(...)` or
  `mark_decorative=True`)

## Canary Examples

Two canary examples in `examples/` exercise the v1 accessibility APIs:

- `examples/semantic_table_a11y_canary/report.py`
- `examples/signature_accessibility_canary/report.py`

Each example emits:

- PDF + preview PNG(s)
- component mount validation JSON
- `A11yContract` validation JSON
