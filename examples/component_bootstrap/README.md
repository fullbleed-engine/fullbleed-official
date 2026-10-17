# Static Bootstrap document kit

This example requires a wheel built from the component preview branch with
`fullbleed.ui.experimental.bootstrap`. It is not in PyPI 2.5.26 or the earlier
`component-preview-f88cced` wheel.

```console
python examples/component_bootstrap/main.py --out target/bootstrap-demo --catalog-pdfs
```

The runner writes a one-page production report, its native PNG preview, separate
HTML/CSS, embedded-style HTML, unstyled HTML, and a catalog covering all 23
Bootstrap 5.0.0 component families plus four layout/content/form/utility groups.
`--catalog-pdfs` also renders each group independently with pagination evidence.

The [report](main.py) uses card content slots, grid columns, data-driven tables,
badges, progress and contextual notes. The [catalog](catalog.py) contains no raw
HTML strings. Fonts are explicitly registered from the bundled assets. Browser
HTML needs an available font or your own `@font-face` rule; font files are not
automatically embedded in HTML exports.

All output is static. HTML uses normal form controls. PDF output uses captured
values and selected states; it is not a fillable form. Inactive panels stay
hidden, overlays appear in flow, and there are no JavaScript dependencies.

See the [API and styling guide](../../docs/component-bootstrap.md) for
`class_name`, style precedence, named card parts, export choices and print
representations. The qualification scripts are:

```console
python -I tools/smoke_component_bootstrap.py --out target/bootstrap-check
python tools/smoke_component_bootstrap_browser.py --out target/bootstrap-check
```

The first requires test-only `pypdf`, `pypdfium2`, Pillow and an installed candidate wheel. The second
requires test-only Playwright/Chromium and opens its own headless context. It
does not attach to your existing browser or generate PDFs through a browser.
