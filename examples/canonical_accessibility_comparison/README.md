# Canonical Accessibility Comparison

Canonical example that generates the same document in two authoring styles:

- `normal`: visually correct, generic UI primitives
- `accessible`: `fullbleed.ui.accessibility` semantic wrappers + signature semantics

Outputs written to `examples/canonical_accessibility_comparison/output/`:

- HTML for both variants
- PDF for both variants
- preview PNGs (if supported by local engine build)
- `A11yContract` validation reports
- component mount validation reports
- comparison report with a11y delta

The normal variant is intentionally "plain authoring" and may produce a11y diagnostics.
The accessible variant is expected to pass strict `to_html(a11y_mode="raise")`.

Run:

```bash
PYTHONPATH=python python examples/canonical_accessibility_comparison/report.py
```
