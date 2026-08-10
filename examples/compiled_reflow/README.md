# Compiled content-reflow example

This example compiles one account-statement template, then renders distinct records whose text and
table lengths naturally change pagination. It demonstrates:

- literal `{{slot}}` bindings for user-controlled scalar text;
- an explicit `data-fb-bind-html="rows"` structural slot built from escaped fields;
- `string-set: ... content(text)` continuation headers;
- a kept heading followed by a splittable long table; and
- per-call throughput or compact compression.

Run either policy from the repository environment:

```powershell
python examples/compiled_reflow/run_example.py --compression throughput
python examples/compiled_reflow/run_example.py --compression compact
```

Outputs are written under `examples/compiled_reflow/output/`. The runner extracts text again and
fails if any record marker is absent.

## Security boundary

Ordinary `{{slot}}` values are literal text and cannot create markup. A `data-fb-bind-html` value is
parsed as trusted HTML. The example's `render_rows` helper calls `html.escape` on every external
field before constructing table rows. In an application that accepts rich user HTML, use an
allowlist sanitizer suitable for your trust model; never feed raw untrusted markup to a structural
slot.
