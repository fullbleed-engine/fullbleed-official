# Repeated page furniture for component authors

Design in progress, 2026-10-18. This describes the next component contract;
the proposed constructors below are **not implemented or released**.

## The authoring task

A report author should be able to add a reusable brand header and a footer
containing its page number, then change the paper size without editing selectors
or manually numbering pages. The same kit should work for an invoice whose
items continue onto a second page. Existing `Header` and `Footer` components
remain ordinary content elements; importing them must not silently repeat them.

The intended entry point belongs to `Page`, which already owns paper geometry.
An explicit page band owns left, center and right slots. A slot accepts an
ordinary immutable component, literal text, or a page-number value. For example,
the proposed shape is:

```python
# Proposed API: this is not an executable example.
Document(
    ReportBody(data),
    page=Page(
        "A4",
        margin=mm(18),
        header=PageBand(left=BrandMark(), right=data.title),
        footer=PageBand(left="Common Ground", right=PageNumbers(total=True)),
    ),
)
```

The goal is to reuse the same content and style values used in the body, while
making repetition an explicit document decision. `PageNumbers` would be a
deferred native value, not a callback that reruns application code during layout.
Its labels must be configurable without introducing a template expression
language. Dynamic page values must not pretend to be ordinary eager text.

## Contracts to settle before exposing constructors

- **Space ownership:** `Page` remains the only owner of paper size and margins.
  A band fits within its reserved margin. It must report an actionable fit error
  when it would overlap the body or another slot. Do not silently scale content,
  clip it, or make body margins depend on a hidden Python measurement algorithm.
- **Rich content:** styles, resources, theme dependencies and source provenance
  travel with a component used in a band. A logo must not require a separate
  stylesheet or another implementation of its component. Slot widths must be
  resolved against the actual page margin area, not the body width at the point
  where a running element was first captured. Theme font choices must reach the
  page context deliberately: setting a font on `body` alone does not set the
  root-inherited font used by generated margin text. The retained fixtures set
  their embedded font on both `html` and `body`.
- **Page selection:** default repetition, first-page suppression and blank-page
  behavior need explicit semantics. Changing a band's content for named or
  facing pages is a later extension unless the initial contract can support it
  without ambiguous precedence.
- **Numbers and records:** native pagination supplies the page and total.
  Direct, compiled, repeated and variable-data output must agree about whether
  those values refer to a record or a combined document. That choice must be
  documented and verified, not inferred from a single-page preview.
- **Accessible output:** repeated navigation belongs outside body reading order.
  Essential text must also appear as real document content. A repeated heading
  must not create extra heading nodes or suppress the body's heading. Verify
  final marked content and independently validate the relevant PDF profiles.
- **HTML export:** styled HTML must retain meaningful content and explicitly
  distinguish screen presentation from paged output. Unstyled HTML must retain
  authored content and classes without fabricated page numbers. Do not promise
  browser print parity for running elements; support differs between engines.
- **Overrides:** band defaults, component defaults and instance styles follow the
  existing immutable style model. Raw `Document(styles=...)` remains available;
  conflicting ownership of an API-managed margin slot needs a diagnostic.

These requirements are acceptance criteria, not current guarantees. In
particular, adding a Python wrapper around `position: running(...)` alone would
not establish fit checking or correct slot sizing.

## Native prerequisite found during implementation

The public 2.5.33 and 2.6.0a7 engines emit generated page-margin text without
marked-content ownership. In a retained three-page probe, six page-number
fixtures fail veraPDF 1.30.3 across PDF/UA-1 and PDF/UA-2. Six body-only and rich
running-element controls pass. The corresponding PDFs from the two releases
are byte-identical. The failing cases have no validator stderr.

The candidate wraps each finalized page-margin box in a balanced artifact
scope. This covers generated text, running-element paint and its decoration,
while leaving document-body tags intact. The correction changes semantic
marking; it does not change pagination or drawing coordinates.
The public-release defect is tracked in [issue #103](https://github.com/fullbleed-engine/fullbleed-official/issues/103).

[`smoke_page_furniture.py`](../tools/smoke_page_furniture.py) checks saved PDFs
with independent readers. Its cases cover every margin position, page counts,
first-page suppression, rich running content, named strings and five emission
paths. CI and the release workflow retain the PDFs, inspections and previews.
The test accepts an older runtime's output directory for independent pixel and
extracted-text comparisons. Separate veraPDF reports are required before
making a PDF/UA claim about the resulting fixtures.

The local installed-wheel candidate passes 1,427 native tests, 11 documentation
tests, 62 focused Python tests, all 940 Python tests and the golden-output
checks. All 120 saved-PDF furniture cases pass; their raster pixels, extracted
text and body-role counts match the public 2.5.33 controls. That public wheel
fails 75 unmarked-content cases and passes 45 controls. All 80 candidate
PDF/UA-1 and PDF/UA-2 fixtures pass veraPDF 1.30.3 with clean stderr. These are
fixture-specific results, not a claim about arbitrary documents or human
assistive-technology testing. CI evidence for the final source commit must be
reviewed separately.

### Existing sizing limitation

Visual review of the `all-margin-boxes` fixture shows overlapping backgrounds
and obscured labels when neighboring margin boxes are all populated. The
current automatic box geometry assigns overlapping regions to some left,
center and right boxes, with a similar problem on the sides. Text extraction
still finds all 16 labels, so that assertion alone cannot establish visible
layout correctness. The candidate preserves those existing pixels; its
artifact correction does not fix sizing. A public `PageBand` API must settle
slot sizing and fit diagnostics before promising that rich neighboring slots
will remain visible.

The local investigation is retained under
`target/growth/component-page-furniture-20261018/`. The installed public release
is the negative control; a source change alone does not establish that a release
contains the correction.

Native behavior follows the [CSS page-margin box model](https://www.w3.org/TR/css-page-3/#page-margin-boxes)
and [running-element model](https://www.w3.org/TR/css-gcpm-3/#running-elements).
Those specifications do not establish Fullbleed's implementation coverage.
