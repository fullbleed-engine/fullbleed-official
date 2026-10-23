# Links in generated PDFs

HTML anchors can provide external URLs and navigation within a document. The
same behavior applies to anchors emitted by Python components.

**Version scope:** this guide describes the unreleased navigation work in
[PR #125](https://github.com/fullbleed-engine/fullbleed-official/pull/125), verified
with candidate wheels. Published `2.6.0a13` predates this work; its version number
alone does not establish support. Use a wheel built from the candidate source
when reproducing these examples. The four-shape observations below record
engine revision `e7ae956`.

## External and internal links

```python
from pathlib import Path
import fullbleed

html = """
<h1>Order summary</h1>
<p><a href="https://example.com/orders/123">View order online</a></p>
<p><a href="#delivery">Delivery details</a></p>
<h2 id="delivery" style="break-before:page">Delivery details</h2>
<p>Your order ships in two packages.</p>
"""
css = """
@page { size: A5; margin: 18mm; }
body { font-family: Helvetica; font-size: 11pt; line-height: 16pt; }
a { color: #146b54; text-decoration: underline; }
"""
Path("order.pdf").write_bytes(fullbleed.PdfEngine().render_pdf(html, css))
```

Use `href="#delivery"` with an element whose `id` is `delivery` for an internal
destination. A missing target produces no active PDF link. An anchor without
`href` is ordinary content. Duplicate IDs resolve to the first source occurrence;
use unique IDs to avoid ambiguity. Legacy `<a name="delivery">` targets are also
recognized, with an `id` taking precedence over a matching legacy name.

Link text can wrap across lines and pages. The output contains separate
activation regions for the visible fragments. Compiled batches keep internal
destinations within their own recipient document.

Empty targets do not add a line box or normal-flow height. Hidden, transparent,
and clipped targets can still identify a destination; `display:none` targets
cannot. Invisible link content does not create an active click region.

## Python components and HTML export

```python
from pathlib import Path
from fullbleed.components import Document, Renderer
from fullbleed.components.bootstrap import Link

document = Document(
    Link("View order online", href="https://example.com/orders/123"),
    title="Order link",
)
Path("component-link.pdf").write_bytes(document.render_pdf(Renderer()))
Path("component-link.html").write_text(document.to_html(), encoding="utf-8")
unstyled_html = document.to_html(include_styles=False)
```

Both HTML exports retain `href`. `include_styles=False` omits component
stylesheets; it does not remove navigation. See
[component authoring](component-authoring.md) for `class_name`, reusable styles,
and the distinction between complete documents and element fragments.

Use descriptive link text. `aria-label` supplies an explicit accessible name
when needed. Tagged link objects and an accessible name are only part of an
accessible document; validate the final file and review its reading and
navigation behavior with the intended assistive technology.

## Reader behavior

The PDF's link geometry and the reader's clickable area are separate concerns.
The candidate emits `Rect` and `QuadPoints` for the painted, clipped regions.
Reader hit testing can still expand a sloped region to a rectangle. In the
measured PDFium and PDF.js versions, rotated and triangular specimens have
clickable points outside their painted shape.

For documents where precise click boundaries matter, keep links rectangular
and avoid placing another control immediately beside a rotated or diagonally
clipped link. Test the final PDF in the readers used by your audience.

The following observations use PDFium **146.0.7665.0**, PDF.js **6.4.299**, and
Chromium **153.0.8010.12**. Counts are mismatching sample points, not percentages
of all possible clicks.

| Authored shape | Sample points | PDF geometry | Painted shape | PDFium hit API | PDF.js hit testing |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rectangle | 1,441 | 0 | 0 | 0 | 0 |
| Rotated rectangle | 1,487 | 0 | 0 | 197 | 197 |
| Triangle | 1,461 | 0 | 0 | 81 | 72 |
| Circle | 1,485 | 0 | 0 | 0 | 0 |

The 5pt sampling grid excludes points within 1.5pt of the authored boundary.
Zero mismatches therefore does not establish exact curve-boundary behavior.
PDF.js checks use its actual annotation layer and confirm selected observations
with mouse clicks. PDFium checks use its link hit-test API. These are not tests
of every browser's PDF viewer, keyboard navigation, or screen-reader behavior.
The [reproduction tools](../tools/pdf_link_readers/README.md) retain the PDFs,
inputs, screenshots, reader versions, hashes, and individual observations. They
return a nonzero result when differences remain.

## Links in imported PDFs

Template composition and stamping also carry link annotations. Cropping a link
requires adjusting its active area while preserving the placement of its
artwork. An explicit PDF appearance stream and a viewer-generated border are
different inputs to this process.

For a partially clipped link, the candidate wraps its original appearance, or
creates one from its border, at the original position. It then disables the
separate border on the clipped annotation. This prevents readers from drawing
an extra edge along the new click boundary. The
[imported-border check](../tools/pdf_link_readers/README.md#check-borders-on-cropped-imported-links)
reproduces this case with independent source PDFs, composition, and stamping.

The reviewed PDF.js specimens preserve explicit appearance streams through
composition and stamping, including artwork whose active region is fully
clipped. When no active region remains, composition retains the original Link
type, sets its ReadOnly flag, and removes actions and destinations. This keeps
the artwork without converting it to a stamp that can open a popup note. A
reader API may still enumerate the retained Link object; it has no active target.

Imported links without appearance streams still show visual differences
for dashed, rounded, beveled, inset, and underline borders. MuPDF's omission of
an ordinary Link appearance is not evidence that an appearance was preserved.

These imported-border differences remain open release work. Do not infer
appearance parity from successful destination checks or from a reader that
does not draw the source annotation. Retaining the stored highlight mode does
not establish identical mouse-highlight behavior across readers. See
[PDF templates](pdf-templates.md) for the composition API and scope.
