# Migrate a complete component document

This example compares the existing `fullbleed.ui` authoring API with the intended
public `fullbleed.components` namespace introduced in **2.6.0a12**. Install it
explicitly with `python -m pip install --pre "fullbleed==2.6.0a14"`, or build this
checkout. The component API remains alpha; the earlier
`fullbleed.ui.experimental` path still exposes the same objects.

[render.py](render.py) contains both complete authoring functions and renders
the same account report on A5 portrait and US Letter landscape paper. Both use
asymmetric margins, an explicitly supplied Inter font, a fixed timestamp and
the same title and language. Neither authoring function calls the other or
converts a serialized HTML string.

```sh
python build_backend/fullbleed_build_backend.py wheel --out target/wheels
# Install the wheel built for your platform, then:
python -I examples/component_migration/render.py --out target/document-migration
```

## Where settings move

| Existing `fullbleed.ui` | New `fullbleed.components` |
| --- | --- |
| `@Document(...)` wraps a function and returns a `DocumentArtifact` | `Document(...)` is a value; ordinary functions return it |
| Actual paper size and margins in stylesheet `@page` or engine options | `Document(page=Page(...))` owns size, orientation and margins |
| Title/language on both artifact and `PdfEngine` | Title/language on `Document`; `Renderer` applies them to each engine |
| Selector rules in `StyleSheet` | Reusable `styled(...)` defaults, explicit `style=`, and `Document(styles=...)` selectors |
| Body typography in stylesheet | `Theme(text=style(...))` supplies document-wide typography |
| `PdfEngine(font_files=..., document_timestamp=...)` | `Renderer(font_files=..., document_timestamp=...)` |
| `artifact.to_html()` and `artifact.to_css()` | `document.to_html(include_styles=True)` for styled browser HTML; `include_styles=False` for content without styling; `compile(target="pdf")` for native print source |

Use `include_styles=True` when the HTML file must carry its stylesheet. The
default `document.to_html()` preserves the separate-CSS form used together
with `document.to_css()`; it does not embed the collected stylesheet.

The old decorator's `page` and `margin` are authoring metadata. Preserve the
actual geometry from the old stylesheet or engine when creating the new `Page`.
Remove size/orientation/margins from migrated `@page` rules; the new API rejects
competing geometry with `styles.page`. Other selectors and page furniture rules
can remain in `Document(styles=...)`. Relative asset paths still need the same
explicit resolution and registrations as before; migrating imports does not
fetch fonts, images or stylesheets.

`class_name="muted"` remains a literal class. Both versions attach a matching
`.muted` rule. A class name alone has no appearance. In the new version the
rule explicitly resolves `token("muted")` from the document's theme.
Unstyled HTML keeps that literal class while omitting generated CSS and inline
style attributes, so a host application can supply its own stylesheet.

## Compatibility

Existing `fullbleed.ui` imports remain supported and are not deprecated.
Earlier alpha imports from `fullbleed.ui.experimental` expose the **same objects**
as `fullbleed.components`; they can compose directly. The static Bootstrap kit
is available at `fullbleed.components.bootstrap` and its earlier path.

For gradual adoption inside an existing document, use `to_legacy(node,
theme=...)`. That bridge converts individual nodes; the existing document keeps
its stylesheet, metadata and engine. It does not convert a whole document or
transfer stylesheet resources such as Bootstrap. See the
[mixed-document example](../component_recipes/README.md#adopt-components-in-an-existing-document).

## Verification

The development-only check runs both authoring paths from an installed wheel:

```sh
python -m pip install --only-binary=:all: -r requirements-test.txt
python -I tools/smoke_component_document_migration.py --out target/document-migration-check
```

It retains both PDFs, native print source, styled and unstyled HTML, independent
PDFium previews and `verification.json`. It checks page count and size, text on
each page, title/language, the resolved engine timestamp, exact preview pixels at
96 dpi, missing glyphs
and repeat bytes within each path. These two explicit migrations do not promise
automatic conversion, arbitrary stylesheet compatibility or PDF conformance.
