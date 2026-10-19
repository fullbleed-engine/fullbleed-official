# Reusable page bands

This **unreleased source example** requires the PageBand implementation on
`feat/component-authoring-prototype`. The published `2.6.0a8` wheel does not
provide these constructors. Build and install this checkout before running it.

```console
python examples/component_page_bands/render.py --out target/page-bands --pdf-profile pdfua2
```

The fictional Common Ground seasonal brief uses ordinary components in its
header and footer. Its three pages have native, finalized page numbers. The
same report exports styled and unstyled browser HTML; those exports show one
header and footer with `[page]` / `[pages]` placeholders. They do not paginate
the browser or promise browser-print parity.

The styled export adds a small screen stylesheet for font loading, a readable
column width and continuous flex layout. `Flow` uses native paginated block
spacing in PDFs; the screen stylesheet keeps its authored row gaps visible in
the browser. It does not change the PDF source or the unstyled export.

Start with [report.py](report.py). `Brand` can also be used in the body or in a
different document. Its styles travel with it. `.folio` demonstrates authored
`class_name` styling through `Document(styles=...)`. `Page` owns the margins;
`PageBand` uses that reserved space and rejects content it cannot prove fits.

The render script registers the bundled Inter font explicitly and checks
deterministic bytes, a three-page result, source semantics, and replay of the
exported HTML/CSS. Those checks alone do not establish PDF/UA conformance or
human accessibility. Inspect the saved PDF with an independent reader, review
every rendered page, and run veraPDF against the requested profile before
making a claim about a particular output. The retained
[qualification record](../../docs/component-page-bands-validation.md) covers
the saved example PDFs, including independent PDF/UA checks for both profiles.

See [the PageBand contract](../../docs/component-page-bands.md) for slot
ownership, style precedence, fit diagnostics and record behavior. The existing
[authored accessible showcase](../accessible_showcase/README.md) remains a
separate example with retained PDF/UA verification evidence.
