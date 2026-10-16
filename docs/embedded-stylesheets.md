# Render a complete HTML document

The 2.5.26 release adds supported static CSS from HTML `<style>` elements.
Check the installed engine before relying on this behavior:

```python
import fullbleed

assert fullbleed.build_features().get("embedded_stylesheets", False)
```

The CLI exposes the same fact as `capabilities.engine.embedded_stylesheets` in
`fullbleed agent-contract --format json`, or `engine.embedded_stylesheets` in
`fullbleed capabilities --json`. Older installed wheels can report false or omit
the field. These repository docs do not establish that a candidate is published.

## One HTML input

```python
from importlib.resources import files
from pathlib import Path
import fullbleed

font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
engine = fullbleed.PdfEngine(font_files=[str(font)])
html = """<!doctype html>
<html lang="en"><head><title>Payment receipt</title>
<style>
  @page { size: A5; margin: 18mm; }
  body { font-family: Inter; font-size: 11pt; color: #263b46; }
  h1 { color: #315d9b; font-size: 28pt; }
  .total { padding: 12pt; background: #edf2f8; font-weight: 700; }
</style></head><body>
  <h1>Payment received</h1>
  <p>Thank you, Maple &amp; Finch.</p>
  <p class="total">Total paid: $120.00</p>
</body></html>"""
Path("receipt.pdf").write_bytes(engine.render_pdf(html, ""))
```

Fonts and other assets remain explicit engine inputs. The same HTML can be passed
to `compile_pdf`, native previews, reporting methods, and batch rendering.

## Source order and cascade

All author sources share the existing CSS cascade, in this source order:

1. CSS assets in the engine's `AssetBundle`, in bundle order.
2. Active embedded stylesheets, in document order, including sheets in the body.
3. The explicit CSS argument supplied to the render or compile call.

Later declarations win only when the cascade otherwise ties. Selector
specificity, `!important`, cascade layers, inheritance, and inline declarations
retain their existing meanings. For example, explicit `.total { color: blue }`
can override an earlier embedded `.total { color: red }`; an embedded `#total`
selector can still outrank that class selector.

Each source is a separate parser input. A malformed sheet cannot swallow the next
sheet through an unfinished comment or block. The native parser can reject an
entire malformed sheet; it does not promise a browser's complete error recovery.
CLI repeatable CSS inputs currently form one explicit CSS argument and therefore
one parsing boundary.

## Which embedded sheets apply

- An absent or empty `type`, or an ASCII case-insensitive `text/css`, is accepted.
  Surrounding whitespace and MIME parameters are not accepted as `text/css`.
- `media` attributes and nested `@media` rules use print media and the engine's
  supported media-query features. `screen` sheets do not style the HTML document.
  This includes the existing `@page`, margin-box, and footnote-area handling.
- HTML style text is raw text: `&amp;` inside a CSS string remains those five
  characters. SVG foreign-content parsing decodes entities and CDATA first.
- Styles inside inert HTML `<template>` content are excluded.
- Untitled sheets are persistent. The first eligible nonempty stylesheet title
  selects the static preferred set; sheets with another title are omitted. An
  earlier HTML `meta http-equiv="default-style"` or preferred non-alternate
  stylesheet link can select that name. There is no script-driven set switching.

Documents with eligible embedded sheets use print media throughout their HTML
style resolution. Documents without such sheets retain the engine's previous
explicit-CSS media behavior, including its screen fallback when no print rule is
present. This feature is not a general browser-layout or CSS-conformance claim.

## Compiled and SVG boundaries

Compilation captures the template's stylesheet sources. Fixed bindings replace
text within the prepared layout; reflow bindings lay out variable content using
the same compiled styles. Structural HTML bindings can insert elements matching
those rules, or carry inline styles. A `<style>` inserted by a structural binding
does not rebuild the document stylesheet. Put shared rules in the template, and
compile a separate template when stylesheet sources change. CSS text and style
attributes are not scalar binding slots.

Inline SVG uses the document cascade for supported paint and font properties
before its subtree is compiled by the native SVG painter. Document class,
ancestor, child, attribute, positional and sibling selectors can style its
descendants. Connected SVG `<style>` elements participate in the same source
order, type, media and preferred-set selection as HTML styles. Their rules are
applied once. SVG presentation attributes precede author stylesheet declarations;
normal inline styles then win unless an important declaration takes precedence.

For example, `.brand svg path {fill:currentColor}` uses the color supplied by
`.brand {color:#d00000}`. Supported paint properties inherit through the HTML
ancestors into the SVG. Registered fonts remain the typography source; no system
font lookup or browser is introduced.

`fill-opacity` and `stroke-opacity` replace inherited or earlier values. A paint
color's own alpha remains separate. `opacity` composites the element or group
once, so two overlapping shapes inside `<g opacity="0.5">` do not darken their
overlap. The outer inline SVG's opacity is applied by its document wrapper once.
Its computed value remains available to children using `opacity:inherit`.
`currentColor` carries the color's alpha into fill and stroke paint. Fully
transparent mask paint contributes nothing; the existing binary luminance-mask
subset does not implement continuous soft-mask transparency.

SVG assets loaded through `<img>` or an asset reference remain isolated from the
containing document's selectors. Their supported local styles use print media
and type/title selection. Fullbleed still implements an SVG subset: this change
does not add browser layout, script execution, animation, arbitrary filters, or
complete SVG/CSS conformance. See the runtime SVG capability map.

Batch outputs retain their single-paper-size restriction. Embedded page settings
initialize streaming writers from the first record; later records must use the
same paper size.

## Diagnostics and verification

External `<link rel="stylesheet">` references and CSS `@import` are not loaded.
Supply their bytes through `AssetBundle` or the explicit CSS argument. JavaScript
is not executed, and a class string does not import a framework stylesheet.

Engine debug asset warnings distinguish unsupported imports (`style-import`)
from a rejected embedded sheet (`style-parse`, with its source index and byte
offset). Other sheets continue to participate in the cascade. The old blanket
warning that every `<style>` is ignored no longer applies.

`tools/smoke_embedded_styles.py` compares installed output with independently
authored explicit-CSS controls. It retains PDFs, PDFium page images, extracted
text, page boxes, and deterministic-repeat checks across direct, compiled,
binding, preview, reporting, and batch paths. These fixtures verify this input
feature; the independent CSS corpus remains a separate broader qualification.

`tools/smoke_svg_styles.py` retains known-color fixtures for document selectors,
inheritance, cascade priority, stylesheet selection, custom properties, gradients,
group opacity and SVG asset isolation. It checks PDFium samples, native previews,
repeat determinism and direct/compiled/fixed/reflow output with forms on/off and
the optional native raster fallback. The expected paint and compositing rules
come from [SVG styling](https://www.w3.org/TR/SVG2/styling.html) and
[SVG group compositing](https://www.w3.org/TR/SVG2/render.html#ObjectAndGroupOpacityProperties).
