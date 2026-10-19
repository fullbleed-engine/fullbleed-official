# Common Ground: an accessible authored showcase

A two-page community-program guide built with reusable Python components, a
local SVG illustration and embedded fonts. All program information is fictional.
The source contains no HTML templates or raw HTML strings.

This source uses the experimental component API in **Fullbleed 2.6.0a8**.
Pin the version supplied with each downloaded source kit; the API may change
before stabilization. Historical downloads retain their original pins.

## Render and edit

```sh
python -m pip install -r requirements.txt
python render.py
```

Open `output/common-ground-guide.pdf`. The output directory also contains a
responsive `guide.html`, semantic `guide-unstyled.html`, the PDF's separate
`source.html` and `source.css`, PNG previews and diagnostic reports. Keep the
exported `fonts/` folder and `garden.svg` beside the HTML files.

The default PDF profile is PDF/UA-1. To render the same design as PDF/UA-2:

```sh
python render.py --pdf-profile pdfua2 --out output-ua2
```

Both authoring paths use the selected profile. `render-report.json` records it.

- `guide.py` composes `ProgramCard`, `ProgramSchedule`, headings, lists and an image.
- `SCHEDULE` supplies the table records. `Column` supplies labels and cell callbacks.
- `guide.css` provides the document design. For example,
  `class_name="program-card"` emits a class selected by `.program-card`; it does
  not discover or load a stylesheet. `Document(styles=...)` loads that CSS.
- `browser.css` adds the responsive browser presentation only. It is deliberately
  excluded from the PDF source. `Page(...)` owns the PDF's A4 geometry.
- `render.py` registers local fonts and artwork and applies the selected PDF profile.

```python
from guide import build_document, ProgramCard

card = ProgramCard("Grow something", "A garden we tend together.")
document = build_document(browser=True)
styled_html = document.to_html(include_styles=True, a11y_mode="raise")
semantic_html = document.to_html(include_styles=False, a11y_mode="raise")
```

The unstyled export keeps semantic elements, text, alt text and authored class
names while removing CSS and inline style declarations. It can be styled by a
different host application.

## Accessibility and verification

The guide authors a document title and language, one H1 and a continuous H2/H3
hierarchy, an informative image description, a captioned table with scoped column
headers, and two semantic lists. Column content remains in authored reading
order. Colors are not used as the only way to identify any information.

`render.py` checks deterministic PDF replay, exact PDF byte parity with the
exported HTML/CSS path, two-page pagination, and native overflow, missing-glyph
and font-substitution gates. `verify.py` uses an independent PDF parser to inspect
the resulting structure tree and text associated with its marked-content IDs:

```sh
python -m pip install -r requirements-check.txt
python verify.py
```

This check also rejects nested marked-content IDs, unbalanced tag boundaries and
duplicate or detached table captions without requiring an external validator.
It checks the caption's text and its place in the table's reading order.
The schedule uses a Strong component
inside its program cells. Alpha 5 carries the correction for the
[table-cell tagging issue](https://github.com/fullbleed-engine/fullbleed-official/issues/97)
that required plain text in the alpha 4 specimen. Edited documents must pass
the same saved-PDF check. Alpha 7 also attaches the schedule caption to its Table
in PDF/UA-2, including correct caption text and reading order across page breaks.

For the PDF/UA-2 output, use the same profile and output directory when checking:

```sh
python verify.py --pdf-profile pdfua2 --out output-ua2
```

For an external standards check, install [veraPDF](https://verapdf.org/software/)
and run the matching profile against the **final** PDF:

```sh
verapdf --format json -f ua1 output/common-ground-guide.pdf
verapdf --format json -f ua2 output-ua2/common-ground-guide.pdf
```

Alternatively, `python verify.py --verapdf-cp PATH_TO_CLASSPATH` runs the
`org.verapdf.apps.GreenfieldCliWrapper` Java entry point. Add
`--pdf-profile pdfua2 --out output-ua2` for that output. The script retains the
profile-specific JSON and stderr and requires a compliant result with empty
stderr. Without the classpath argument, it explicitly records that external
validation did not run. Java and the independent readers are verification tools,
not dependencies of the document renderer.

Validation receipts identify the exact PDF by SHA-256. Review both rendered
pages along with their tag order, image description and table header scope.
These checks are **not a screen-reader user test or blanket WCAG certification**.
Changed content or styling needs fresh rendering, inspection and validation.

Two current runtime boundaries are visible in this example: HTML links become
plain PDF text, so the destination is also written out; and decorative paragraph
text cannot be reliably excluded using HTML ARIA attributes in this pinned alpha,
so redundant folios and card numbers are omitted. The exported HTML retains its
descriptively named, keyboard-focusable link.

The composition has a fixed two-page print layout. It is a showcase to adapt,
not an arbitrary-length program-directory template.

## Assets and license

Python, CSS and SVG source are MIT licensed. DM Serif Display is vendored under
the SIL Open Font License; Inter comes from the pinned Fullbleed wheel and is
copied beside the HTML for browser use. Both font licenses are in `fonts/`.
The renderer records the exact font hashes in `output/render-report.json`.
