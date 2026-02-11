<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# Public Golden Suite

This directory is the public golden regression suite for Fullbleed output parity.

It ships three customer-facing fixtures:

- `invoice`: transactional invoice layout
- `statement`: account statement with running balances
- `menu`: styled restaurant menu with two-column hierarchy

Each case has:

- Source inputs under `goldens/cases/<case>/`
- Committed expected PNG baselines under `goldens/expected/png/<case>/`
- Committed expected hash contract in `goldens/expected/golden_suite.expected.json`

## Why This Exists

The suite gives launch and CI a stable answer for:

- Did this change alter rendered PDF bytes?
- Did the first-page visual output change?
- Did the CLI still emit expected artifacts (PDF + page PNG)?

## Run

Generate/update expected baselines:

```bash
python goldens/run_golden_suite.py generate --cli "python -m fullbleed"
```

Verify against committed expected hashes:

```bash
python goldens/run_golden_suite.py verify --cli "python -m fullbleed"
```

Run a single case:

```bash
python goldens/run_golden_suite.py verify --case invoice --cli "python -m fullbleed"
```

CLI resolution:

- If `--cli` is omitted, the harness tries `fullbleed` from `PATH`.
- If that is unavailable, it falls back to `python -m fullbleed` when importable.

## Output Layout

Ephemeral render outputs (ignored by git):

- `goldens/output/generate/<case>/`
- `goldens/output/verify/<case>/`

Committed expected artifacts:

- `goldens/expected/golden_suite.expected.json`
- `goldens/expected/png/invoice/invoice_page1.png`
- `goldens/expected/png/statement/statement_page1.png`
- `goldens/expected/png/menu/menu_page1.png`

