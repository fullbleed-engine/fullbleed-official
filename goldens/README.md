# Golden Suite

This suite validates the component-driven examples created from `fullbleed init`
and treated as a contract for engine-safe primitives/selectors.

## Coverage

- `acme_invoice` (scaffolded component example)
- `bank_statement` (scaffolded component example)
- `coastal_menu` (component showcase example)

For scaffolded examples, the suite enforces:

- Component mount validation is clean (`ok: true`, no warnings/failures)
- Missing glyphs/overflow/CSS warnings/known-loss/html-asset warnings are zero
- CSS layer safety contract is clean:
  - `unscoped_selector_count == 0`
  - `no_effect_declaration_count == 0`

For all examples, the suite also verifies:

- PDF hash
- Per-page PNG hash set
- Checked-in PNG baselines under `goldens/expected/png/<case>/`

## Commands

Generate expected artifacts/hashes:

```bash
python goldens/run_golden_suite.py generate
```

Verify against expected artifacts/hashes:

```bash
python goldens/run_golden_suite.py verify
```

Use a specific interpreter/venv:

```bash
python goldens/run_golden_suite.py --python .venv_engine/Scripts/python.exe verify
```

Run a subset:

```bash
python goldens/run_golden_suite.py --cases acme_invoice,bank_statement verify
```

