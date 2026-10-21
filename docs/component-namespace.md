# Component imports and migration

The intended public import path for the new 2.6 component API is
`fullbleed.components`, with its static Bootstrap kit at
`fullbleed.components.bootstrap`. **The API remains alpha.** This namespace is
available in this source checkout; the published `2.6.0a11` wheel uses the
earlier `fullbleed.ui.experimental` path. Build and install this checkout to try
the new path before the next alpha is published.

Existing `fullbleed.ui` imports remain supported. The namespace change does not
deprecate them or replace their `Document` decorator. Kit-composition conventions
and independent author feedback remain separate stabilization gates; see the
[current release decision](https://docs.fullbleed.dev/guides/component-status/).

## Coming from an earlier alpha

Change the import prefix:

```python
# Earlier alpha:
from fullbleed.ui.experimental import Document, Paragraph, Renderer
from fullbleed.ui.experimental import bootstrap as bs

# Intended public namespace:
from fullbleed.components import Document, Paragraph, Renderer
from fullbleed.components import bootstrap as bs
```

Both paths export the same classes and functions. Components, styles, themes,
pages, errors and Bootstrap helpers can be mixed across those two paths. There
is no second style registry or stylesheet copy. Generated re-export stubs retain
the original signatures, accepted values and editor completions.

This is an import migration for the newer API. It does not turn the existing
`fullbleed.ui.Document` decorator into the newer document value.

## Coming from the existing component API

Use the [complete migration example](../examples/component_migration/README.md).
Its paired implementations show how to move an existing report's styles, paper
geometry, title, language, explicit fonts and renderer options. Both paths keep
the same A5 portrait and Letter landscape output, including asymmetric margins
and pagination, in the installed-wheel qualification.

The new `Document` owns title, language, page and theme. `Page` owns size,
orientation and margins. `Renderer` receives fonts, assets and engine options;
it creates an engine for each document and applies that document's metadata.
Do not put conflicting page geometry in `Document(styles=...)` or document-owned
metadata in `Renderer(...)`.

Styles can move progressively. `Document(styles=...)` accepts selector CSS or a
`StyleSheet`, so a literal `class_name` keeps working when its matching rule is
attached. Reusable defaults can move into `styled(...)` or a component's own
`style`. Document-wide typography belongs in `Theme(text=style(...))`. None of
these paths fetches CSS or fonts implicitly.

For mixed old/new trees, `to_legacy(node, theme=...)` adapts one new component
into the old tree. The old document still owns its stylesheet, geometry,
metadata and engine. The adapter does not convert whole documents or transfer
resource-owning kits such as Bootstrap. Import changes alone cannot migrate an
old `DocumentArtifact` into a new `Document`.

## HTML output

`document.to_html()` includes the new document's component styles, theme and
attached selectors. `document.to_html(include_styles=False)` emits content
without generated CSS or inline styles; literal classes remain available to
your host stylesheet. Use `compile(target="pdf")` for the native print source.
Continuous browser HTML and PDF pagination are separate output contexts.

## Retained checks

The CI and release workflows run both namespaces through the installed-wheel
mypy and Pyright checks, including actual language-server completions. The
whole-document example retains independent PDF text and pixel comparisons;
the existing node-migration checks remain in place. Source tests also verify
mixed namespace composition and the continued legacy decorator/adapter path.

These checks cover the documented examples and contracts. They do not establish
automatic migration of arbitrary stylesheets, independent kit usability or
human accessibility results, and do not graduate the API from alpha.
