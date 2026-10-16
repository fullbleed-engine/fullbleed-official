# Render a complete HTML document

The source candidate applies supported static CSS from HTML `<style>` elements.
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

Inline SVG remains a leaf rendered by the existing SVG subset pipeline. Its
connected `<style>` elements also contribute to the HTML cascade, but HTML
selector rules are not propagated into SVG descendants. The SVG renderer has
separate local stylesheet handling, including existing differences in `type`,
`media`, and preferred-set filtering. Use explicit SVG presentation attributes or
a tested self-contained SVG asset when those distinctions matter. Consult the
runtime SVG capability map; this change does not establish shared HTML/SVG
stylesheet parity.
The shared-cascade work is tracked in [issue 87](https://github.com/fullbleed-engine/fullbleed-official/issues/87).

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
