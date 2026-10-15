# An invoice built from Python components

Reusable functions, named elements, props, children, data-driven table rows,
conditional content and Python style mappings. No authored HTML is needed.

```sh
python -m pip install fullbleed==2.5.25
python examples/components/invoice.py
```

Open `examples/components/output/invoice.pdf` or `page-1.png`. The output folder
also contains generated HTML/CSS and a repeat-render receipt. The font comes
from the installed package. All business data is fictional.

- `invoice.py`: `Field`, `WorkRow`, `WorkTable`, `PaymentNote`, `Total` and `Invoice`.
- `styles.py`: reusable `StyleSheet` with print dimensions and visual design.
- `data.json`: customer, items, integer-cent rates and payment status.

Set `paid` to true, add rows, or override the components. See the
[component guide](../../docs/component-authoring.md) for composition and styles.
The installed-package check in `tools/smoke_component_authoring.py` exercises
default, escaped, paid, empty and long-table inputs plus standalone layouts.
It checks PDF text, repeated bytes, missing glyphs, preview counts and basic
geometry. Structural validation does not establish PDF accessibility conformance.
