# Semantic Table A11y Canary

Canary example for `fullbleed.ui.accessibility` table/field semantics.

Exercises:

- `FieldGrid` / `FieldItem` (`dl` / `dt` / `dd`)
- `SemanticTable*` wrappers + header scopes
- `Region` labeling
- `A11yContract` validation
- tagged PDF rendering path (for `th scope` propagation)

Run:

```bash
PYTHONPATH=python python examples/semantic_table_a11y_canary/report.py
```
