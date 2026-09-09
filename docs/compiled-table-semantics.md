# Compiled table spans

Table layout emits resolved positive `column_span` and `row_span` evidence on
each real TH/TD `Command::BeginTag`. Values describe the emitted page fragment:
collapsed columns/rows do not count, `rowspan=0` has already been resolved to the
authored row group, and repeated headers carry their spans on every page.
Rowspan placeholders never become extra tagged cells. Authored TBody boundaries
survive compilation, including groups whose first row is collapsed.

`AuthoringReadingNode` exposes these optional fields without changing
`fullbleed.authoring_reading_preview.v1`. None means unavailable evidence; it
must not be advertised as a known one-cell span. Existing `column_index` remains
the logical source-grid column, not a renumbered visible-grid index. Python
accessibility trace sample events expose the same spans; samples remain bounded
observations, not a complete table index.

`Canvas::begin_tag` stays callable with its existing arguments and emits no span
evidence. `begin_table_cell_tag` accepts resolved `(columns, rows)` spans. Spill
opcode 32 remains byte-compatible for legacy tags; opcode 52 appends the optional
spans. Old spills load with unavailable spans rather than an invented default.

## PDF semantics and boundaries

The PDF writer emits non-default `ColSpan`/`RowSpan` and supported header `Scope`
inside a Table-owner attribute dictionary: `/A << /O /Table ... >>`. These are
semantic changes to tagged PDF bytes, not painting commands. Untagged PDF and
Fullbleed PNG bytes are unchanged by adding spans.

Supported PDF Scope names are Row, Column and Both. HTML rowgroup/colgroup and
unknown source scope names are retained in compiler observations but are not
written as invalid PDF Scope names. Resolving those group-specific associations,
and explicit HTML `headers`/ID references, remains separate work. The old
automatic first-header-in-column guess is removed: a standard PDF Headers array
requires byte-string structure IDs, not indirect object references. Do not infer
complex-table accessibility or PDF/UA conformance from spans alone.

References: [PDF Association structure attributes](https://pdfa.org/download-area/cheat-sheets/StructureAttributes.pdf),
[ISO 32000 attribute-dictionary clarification](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html#14761-general),
and [HTML table attributes/model](https://html.spec.whatwg.org/multipage/tables.html#attributes-common-to-td-and-th-elements).

Regression coverage exercises ID-free source, row groups, collapsed rows/columns,
repeated page headers, old/new spill serialization, unchanged painting, and
installed direct/compiled/repeated/reflow tagged PDF emission. The existing
fixed-geometry variable-binding API explicitly refuses tagged structure; this
refusal is tested, not silently converted to untagged output. Compiled reflow is
the available tagged variable-binding path. Manual assistive technology testing
and external conformance validation are separate acceptance.
