# Compiled table semantics

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
written as invalid PDF Scope names. Explicit HTML `headers`/ID references resolve
to emitted TH structure IDs within the same logical table. The IDs are scoped to
each output record, including repeated and reflow batches. Missing, ambiguous,
cross-table and unrendered targets produce structured diagnostics; the engine
does not guess associations from nearby text or silently bind another table's
header. A standard PDF Headers array contains byte-string structure IDs, not
indirect object references. Do not infer complex-table accessibility or PDF/UA
conformance from spans or header links alone.

## Captions

A native HTML caption or a CSS `display:table-caption` belongs to its containing
Table in both the authoring reading tree and tagged PDF. The layout wrapper owns
the Table group, with one Caption containing paragraph content and the table's
rows beside it. `caption-side:top` places Caption first; `caption-side:bottom`
places it last. Nested and adjacent tables keep separate caption owners.

The source table and caption identities survive pagination. A caption that wraps
onto several pages remains one logical Caption, retaining every page's text
references in order. Batch records receive separate identities. Empty or hidden
caption boxes retain their existing layout behavior without adding an empty
Caption to the reading tree. This does not add JavaScript or change component
authoring syntax: `DataTable` captions use the same HTML compiler path.

These rules follow the [PDF caption structure requirements](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html#148484-caption-structure-types).
They are structural guarantees, not a blanket accessibility claim. Validate the
actual saved PDF with the required profile and review its authored semantics.

`python -I tools/smoke_table_captions.py --out target/table-captions` retains
PDFs, source HTML/CSS, independent structure/text inspection and PDFium previews
for native/CSS, top/bottom, nested, adjacent, paginated and empty/hidden captions.
It covers direct, compiled, repeated, reflow and compact reflow output with
deterministic replay and untagged paint comparisons. `--baseline PATH` additionally
compares saved-PDF pixels, extracted page text and untagged bytes to a prior run.
Run independent PDF/UA validation separately and retain its JSON and stderr.

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
