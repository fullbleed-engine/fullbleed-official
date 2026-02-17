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
2. compose result reports `pages_written=10`
3. binding contracts match expected template ids for each page
4. both authoring paths (`raw` and `el()`) produce identical binding outcomes
