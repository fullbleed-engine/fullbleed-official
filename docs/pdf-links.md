# Links in generated PDFs

HTML anchors can provide external URLs and navigation within a document. The
same behavior applies to anchors emitted by Python components.

**Version scope:** this guide targets the **2.6.0a14** prerelease, which
packages [PR #125](https://github.com/fullbleed-engine/fullbleed-official/pull/125).
Stable `2.5.34` and alpha `2.6.0a13` predate these native-navigation changes.
For an unpublished checkout, build and install its wheel before reproducing
the examples. The retained reader observations identify their exact source
revisions and native modules; the original four-shape observations use `e7ae956`.

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

When a cropped source link has no appearance stream, Fullbleed creates one
from its declared border dimensions and style. This fixes the artwork's
position before clipping. It can change how the border looks compared with
the original reader-generated border. For imported artwork whose exact
appearance matters, supply an explicit appearance stream and check the final
PDF in the intended readers.

The [border geometry check](../tools/pdf_link_readers/README.md#check-declared-border-geometry)
measures the generated appearances against independent source dimensions.
Across eight axis-aligned specimens, composition and stamping produce the
same pixels. The solid, underline, bevel, and inset masks have no differences
at 63,000 sampled points. The check also verifies the declared 3pt stroke,
4pt/2pt dash pattern, sampled rounded corners, opposed bevel/inset shading,
and the absence of an extra edge at the crop. These observations do not
establish exact curve rasterization or general imported-PDF parity.

One difference is independently reproducible before composition: the official
PDF.js 6.4.299 viewer and its 6.5.316 online demo keep a source link's 3pt border
at 3 CSS pixels as the page zoom changes. The
[small source PDF and measurements](../tools/pdf_link_readers/README.md#inspect-a-source-link-border-at-different-zoom-levels)
compare that annotation with a 3pt page stroke. Fullbleed does not generate
this control, so this observation identifies a reader-side scaling difference;
it does not establish border preservation by Fullbleed.

The retained [source-only controls](../tools/pdf_link_readers/README.md#source-reader-differences)
also show why a reader's fallback pixels are not a universal reference.
PDFium 146 omits the six border-only controls while drawing the explicit
appearance control. PDF.js 6.4.299 omits bevel/inset borders and renders two
different dash arrays identically. Poppler 22.02.0 draws the solid, rounded,
beveled, and inset controls identically, with paint extending outside their
declared rectangles. These are measured compatibility limits; the release
does not promise to reproduce each reader's fallback behavior.

Do not infer appearance parity from successful destination checks or from a
reader that does not draw the source annotation. Retaining the stored highlight
mode does not establish identical mouse-highlight behavior across readers. See
[PDF templates](pdf-templates.md) for the composition API and scope.
