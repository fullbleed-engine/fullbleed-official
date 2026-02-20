# CSS Parity Canonical (100 Records)

Deterministic, rich HTML/CSS fixture for parity and performance checks in one run.
Default scale is tuned for benchmark mode: 100 pages total.

## What It Exercises

- 1400 generated records (seeded and stable, 14 per page -> 100 pages)
- Multi-page layout with `break-after: page`
- CSS custom properties (`var(...)`) and fallback
- `calc(...)`, `min(...)`, `max(...)`, `clamp(...)`
- Flex + grid layout in the same page
- Absolute positioning, transforms, shadows, gradients, rounded corners
- Table layout and repeated card structures for perf pressure
- Optional debug and perf traces from `PdfEngine`
- Explicit vendored `Inter` font registration (`vendor/fonts/Inter-Variable.ttf`) to keep text spacing stable

## Run

```powershell
python examples/css_parity_canonical_100/run_example.py
```

Outputs are written to:

- `examples/css_parity_canonical_100/output/css_parity_canonical_100.pdf`
- `examples/css_parity_canonical_100/output/css_parity_canonical_100_page*.png`
- `examples/css_parity_canonical_100/output/css_parity_canonical_100.run_report.json`
- `examples/css_parity_canonical_100/output/css_parity_canonical_100.perf.jsonl` (enabled by default)

## Environment Controls

- `FULLBLEED_PERF=0|1` (default: `1`)
- `FULLBLEED_DEBUG=0|1` (default: `0`)
- `FULLBLEED_JIT_MODE=plan|layout|paint` (optional)
- `FULLBLEED_IMAGE_DPI=120` (default: `120`)
- `FULLBLEED_EMIT_PNG=0|1` (default: `1`)
- `FULLBLEED_EMIT_PAGE_DATA=0|1` (default: `0`)
