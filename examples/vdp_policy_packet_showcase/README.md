# VDP Policy Packet Showcase

This is a non-smoke, production-style VDP showcase for PDF template composition.

It exercises a hard workload:

1. Large record batch (`--records`, default `1000`).
2. Variable per-record page counts (front, detail overflow, inserts, coupon, back).
3. Conditional state inserts (`CA`, `NY`, `TX`, `FL`).
4. Conditional back-page routing (`blank`, `legal`, `marketing`).
5. Duplex parity alignment with explicit parity-blank pages.
6. Template routing via `data-fb` features and Rust finalize compose.
7. Determinism pass over repeated runs (structural signature stability).
8. PDF template assets validated through `vendored_asset(..., "pdf")` and `AssetBundle`.

## What The Runner Does

`run_showcase.py` performs the full workflow end-to-end:

1. Vendors `Inter` via CLI:
   - `python -m fullbleed assets install inter --vendor ... --json`
2. Authors all template PDFs with the Fullbleed engine.
3. Generates deterministic synthetic VDP records.
4. Builds overlay HTML/CSS with per-page feature flags.
5. Runs canonical CLI auto-compose:
   - `render --templates ... --template-binding ...`
6. Validates composed output (template marker + overlay marker).
7. Verifies duplex start parity and deterministic signatures.
8. Enforces production-style CLI gates:
   - `--fail-on font-subst`
   - `--fail-on budget --budget-max-pages <expected>`

## Run

Quick verification:

```bash
python examples/vdp_policy_packet_showcase/run_showcase.py --records 120 --runs 2
```

Hard pass:

```bash
python examples/vdp_policy_packet_showcase/run_showcase.py --records 1000 --runs 3
```

Optional debug artifacts on run 1:

```bash
python examples/vdp_policy_packet_showcase/run_showcase.py --records 1000 --runs 3 --emit-debug
```

## Outputs

Generated under `examples/vdp_policy_packet_showcase/output/`:

- `policy_packet_showcase.pdf`
- `showcase_report.json`
- `records.jsonl`
- `showcase_overlay.html`
- `showcase_overlay.css`
- `template_binding.json`
- `oracle_first20.json`

Template assets are generated under:

- `examples/vdp_policy_packet_showcase/assets/templates/`

Note:
- This showcase does not currently enable `--fail-on overflow` because overflow gating depends on `jit.docplan` signals that are not emitted in the current auto-compose render path.
