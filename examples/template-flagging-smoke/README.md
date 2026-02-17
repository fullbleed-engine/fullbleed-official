# Template Flagging Smoke

Purpose:
- validate feature-based template binding for a 10-page job
- prove both authoring paths work:
  - raw HTML
  - `el()` wrapper

What it generates:
1. `output/rgb_template_3pages.pdf`
  - page 1: blue square
  - page 2: red square
  - page 3: green square
2. 10-page overlay PDFs (raw + `el()`)
3. 10-page composed PDFs (raw + `el()`)
4. `output/smoke_report.json`

Run:

```bash
python examples/template-flagging-smoke/run_smoke.py
```

Validation checks:
1. template binding decisions match expected page sequence
2. composed output page count is 10
3. each composed page center pixel matches expected template color
4. overlay marker text remains present on each page (format/path sanity)
