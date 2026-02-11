# Living Docs Atlas

This is a fullbleed init-style project used as living documentation for:

- Engine usage (`report.py`)
- CLI usage (`scripts/run_cli_pipeline.py`)
- Curated font coverage (33 remote packages from `data/font_catalog.json`)
- Major Bootstrap patterns (layout, grid, components, forms, utilities)

It is designed for both human readers and AI agents.

## Project Shape

This example keeps the same core scaffold shape as `fullbleed init`:

- `report.py`
- `assets.lock.json`
- `templates/`
- `vendor/`
- `output/`

Additional docs-focused files:

- `data/font_catalog.json`: source of truth for font packages/filenames/licenses.
- `data/bootstrap_features.json`: major Bootstrap feature map represented in the docs.
- `data/language_samples.json`: multilingual specimen copy used to generate per-font pages.
- `scripts/build_inputs.py`: generates `build/atlas.html` and `build/atlas.css`.
- `scripts/install_assets.py`: installs Bootstrap + all curated fonts to `vendor/`.
- `scripts/run_cli_pipeline.py`: runs `fullbleed render` + `fullbleed verify`.

## Quick Start

From this directory:

```powershell
python scripts/install_assets.py
python scripts/build_inputs.py
python scripts/run_cli_pipeline.py
python report.py
```

Fast smoke path (Bootstrap only):

```powershell
python scripts/install_assets.py --limit 1
python scripts/build_inputs.py
python scripts/run_cli_pipeline.py
```

Outputs:

- CLI PDF: `output/living_docs_atlas.cli.pdf`
- Engine PDF: `output/living_docs_atlas.engine.pdf`
- Verify PDF: `output/living_docs_atlas.verify.pdf`
- Input artifacts: `build/atlas.html`, `build/atlas.css`
- Multipage font registry section with repeating table headers in paged output.
- Per-font specimens: `build/specimens/*.html`, `build/specimens/*.css`, `build/specimens/index.html`
- Machine summaries: `build/*.json`

Render one font specimen through CLI:

```powershell
fullbleed --json render --html build/specimens/inter.html --css build/specimens/inter.css --css vendor/css/bootstrap.min.css --out output/inter.specimen.pdf
```

## Human Workflow

1. Install/update assets with `python scripts/install_assets.py`.
2. Inspect generated input files in `build/`.
3. Run CLI pipeline and compare `render`/`verify` JSON summaries.
4. Run engine path via `python report.py` for parity check.

## AI Workflow

Stable machine-readable artifacts:

- `build/atlas.summary.json`
- `build/assets_install_report.json`
- `build/cli_pipeline_summary.json`
- `build/engine_render_summary.json`
- `build/specimens/specimens.manifest.json`

Contract expectation:

- `font_total` should remain equal to `data/font_catalog.json` count.
- `font_missing` should trend to `0` in fully provisioned environments.
- CLI and engine PDFs should both be emitted on successful runs.
- `font_specimen_count` should equal `font_total` when specimen generation is enabled.

## Notes

- This example intentionally keeps vendor binaries out of git by default.
- If `vendor/css/bootstrap.min.css` is absent, `report.py` falls back to bundled Bootstrap when available.
- `scripts/run_cli_pipeline.py` includes compatibility handling for environments where older CLI builds do not expose every newer global flag.
- `scripts/install_assets.py` includes a catalog-download fallback for fonts so provisioning still succeeds with older global CLI binaries.
- Per-font pages use multilingual text to validate non-English shaping and fallback behavior.
