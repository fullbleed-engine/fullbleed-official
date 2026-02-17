# Form I-9 Canonical Composition Example

This example uses a vendored source PDF (`i-9.pdf`) and overlays variable data using the component authoring model, then composes the final output through Rust finalize.

## What This Shows

- `fullbleed init` scaffold structure (components + layered CSS + vendored assets)
- first-class PDF template asset registration/validation
- deterministic overlay rendering from canonical component code
- Rust `finalize_compose_pdf` template composition path
- field geometry/data separation:
  - `data/i9_field_layout.json`
  - `data/data.json`

## Run

```bash
python report.py
```

Default example watermark:
- Output is watermarked with `EXAMPLE` by default.
- To disable it:

```bash
$env:FULLBLEED_I9_WATERMARK=0
python report.py
```

Optional custom watermark text:

```bash
$env:FULLBLEED_I9_WATERMARK="INTERNAL USE ONLY"
python report.py
```

Size optimization default:
- This example does **not** embed Inter by default, so large VDP jobs avoid per-record font duplication.
- To force embedded Inter for metric-compat testing, set:

```bash
$env:FULLBLEED_I9_EMBED_INTER=1
python report.py
```

## Permutation VDP Proof Job

Generate a large record matrix and compose a single merged VDP output:

```bash
python run_vdp_permutation_job.py
```

This emits:
- `output/permutation_vdp/composed_merged.pdf`
- `output/permutation_vdp/overlay_merged.pdf`
- `output/permutation_vdp/manifest.json`
- `output/permutation_vdp/records.json`

Note:
- The permutation runner now uses a Fullbleed-only contract (no third-party PDF parser).
- Default chunk size is large so the canonical run emits single merged artifacts directly.

Current matrix categories:
- baseline: 1 record
- checkbox permutations: 256 records (`2^8`)
- combo sweeps: 265 records (5 combo fields across state code domain)
- text variants: 351 records (per-field `blank`, `maxfit`, `alternate`)

## Output Artifacts

- `output/i9_overlay.pdf`: rendered overlay only
- `output/report.pdf`: composed final PDF (overlay + `i-9.pdf`)
- `output/template_bindings.json`: per-page template binding decisions
- `output/compose_report.json`: finalize compose summary
- `output/template_asset_validation.json`: PDF asset metadata/validation report
- `output/field_fit_validation.json`: Fullbleed-only heuristic field-fit validation
- `output/component_mount_validation.json`: component mount smoke report
- `output/report_page_data.json`: page data payload from render step
- `output/report_page*.png`: overlay preview PNGs from engine image rendering

## Field Data Contract

`data/data.json` uses stable, human-readable keys such as:
- `p01_last_name_family_name`
- `p01_section1_status_us_citizen`
- `p03_preparer_state_0`
- `p04_reverification_row_2_alt_procedure_used`

Keys map to exact widget geometry in `data/i9_field_layout.json`.

## Regenerate Layout/Data From PDF

Refresh seeded values from canonical layout:

```bash
python tools/build_i9_fields.py
```

Note:
- This tool intentionally avoids third-party PDF parsers. Keep `data/i9_field_layout.json` as the canonical checked-in layout input.
