# CSS Visual Charts Showcase

Pure HTML/CSS chart stress fixture intended to push visual parity coverage.

## What It Exercises

- Layered gradients and chart-style panel composition
- Grid + flex orchestration under dense node counts
- Circular gauge cards + progress strips
- Heatmap matrix and timeline milestones
- 72 deterministic records rendered as compact metric cards
- Vendored font path (`vendor/fonts/Inter-Variable.ttf`) for stable text metrics

## Run

```powershell
python examples/css_visual_charts_showcase/run_example.py
```

## Outputs

- `examples/css_visual_charts_showcase/output/css_visual_charts_showcase.pdf`
- `examples/css_visual_charts_showcase/output/css_visual_charts_showcase_page*.png`
- `examples/css_visual_charts_showcase/output/css_visual_charts_showcase.run_report.json`
- `examples/css_visual_charts_showcase/output/css_visual_charts_showcase.perf.jsonl` (default on)

## Environment

- `FULLBLEED_PERF=0|1` (default: `1`)
- `FULLBLEED_DEBUG=0|1` (default: `0`)
- `FULLBLEED_EMIT_PNG=0|1` (default: `1`)
- `FULLBLEED_IMAGE_DPI=132` (default: `132`)
