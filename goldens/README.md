# Golden Suite

This suite validates the component-driven examples created from `fullbleed init`
and treated as a contract for engine-safe primitives/selectors.

## Coverage

- `acme_invoice` (scaffolded component example)
- `bank_statement` (scaffolded component example)
- `coastal_menu` (component showcase example)

## Pending canary enrollment

The repository now also includes accessibility-focused canary examples that are
intended for future golden enrollment once baselines are generated and the suite
contract is extended:

- `examples/semantic_table_a11y_canary/report.py`
- `examples/signature_accessibility_canary/report.py`

These canaries emit both component-mount validation JSON and `A11yContract`
validation JSON and are good candidates for Sprint 6/7 golden expansion.

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

## Reviewed baseline changes

The 2.5.30 inline line-height correction moves only Acme's large heading up
0.75 points. Its 54px Inter, weight 800, `line-height: 1` baseline is now 72pt,
matching a Chrome 155 saved PDF using the same bundled font. At the golden's
144-DPI size, all changed pixels lie within `(69, 82, 542, 148)` on the
1224-by-1584 page. The remaining invoice content and the bank-statement and
coastal-menu baselines are unchanged. The installed inline-line-height gate
also retains independent controls for this font size and weight.

The navigation and inline-shaping update refreshes Acme and bank-statement
baselines after reviewing the same authored HTML/CSS with the published alpha,
the source candidate, and an independent Chromium 153 print reference. Text,
vertical glyph positions, and page counts remain unchanged. Acme changes 791
pixels: nine invoice-number glyphs move left by 0.025pt, and the four glyphs in
`(8%)` move left by 0.904pt because the preceding word now advances by its shaped
width. With Inter's default axes fixed at `opsz=14, wght=400`, the tax label's
maximum relative horizontal glyph error against the browser reference falls
from 0.924pt to 0.020pt.

The bank statement changes 1,664 pixels in its three contact rows. Their 58 text
glyphs move horizontally by at most 0.048pt as intrinsic widths use shaped
advances; page two remains byte-identical. The other golden case is unchanged.
This is a review of these specific spacing changes, not a claim that the full
examples match browser layout. CI retains the generated PDFs, previews, and
diagnostics even when a golden comparison fails.
