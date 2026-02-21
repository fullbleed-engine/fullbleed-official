# MDN CSS Parity Inventory Snapshot

Source taxonomy: https://developer.mozilla.org/en-US/docs/Web/CSS

Generated from `_css_working/css_parity_status.json`.

## Overall

- Total modules: `22`
- In progress: `19`
- Not started: `3`
- Implemented: `0`
- Module progress: `86.36%`

## Stage Rollup

- parser: `19/22` modules progressed (`86.36%`) | implemented `0`
- compute: `19/22` modules progressed (`86.36%`) | implemented `0`
- layout: `13/16` modules progressed (`81.25%`) | implemented `0`
- paint: `7/10` modules progressed (`70.00%`) | implemented `0`

## Module Matrix

| # | Module | Status | Parser | Compute | Layout | Paint |
|---|---|---|---|---|---|---|
| 1 | Syntax and at-rules | in_progress | partial | partial | n/a | n/a |
| 2 | Selectors | in_progress | partial | partial | n/a | n/a |
| 3 | Cascade and inheritance | in_progress | partial | partial | n/a | n/a |
| 4 | Values and units | in_progress | partial | partial | n/a | n/a |
| 5 | Box model | in_progress | partial | partial | partial | partial |
| 6 | Display and formatting contexts | in_progress | partial | partial | partial | n/a |
| 7 | Positioning | in_progress | partial | partial | partial | n/a |
| 8 | Sizing | in_progress | partial | partial | partial | n/a |
| 9 | Text and fonts | in_progress | partial | partial | partial | partial |
| 10 | Backgrounds and borders | in_progress | partial | partial | partial | partial |
| 11 | Lists and counters | in_progress | partial | partial | partial | partial |
| 12 | Overflow | in_progress | partial | partial | partial | partial |
| 13 | Flexbox | in_progress | partial | partial | partial | n/a |
| 14 | Grid | in_progress | partial | partial | partial | n/a |
| 15 | Tables | in_progress | partial | partial | partial | partial |
| 16 | Transforms and coordinate spaces | in_progress | partial | partial | n/a | partial |
| 17 | Filters/effects/compositing | not_started | not_started | not_started | not_started | not_started |
| 18 | Multi-column | not_started | not_started | not_started | not_started | not_started |
| 19 | Fragmentation | in_progress | partial | partial | partial | n/a |
| 20 | Paged media | in_progress | partial | partial | partial | n/a |
| 21 | Writing modes and logical properties | not_started | not_started | not_started | not_started | not_started |
| 22 | Custom properties and API-adjacent parsing | in_progress | partial | partial | n/a | n/a |

## Not Started Modules

- `filters_effects_compositing` (Filters/effects/compositing)
- `multi_column` (Multi-column)
- `writing_modes_logical` (Writing modes and logical properties)

## Validation State

- Full fixture lane: `62/62` passing (`tools/run_css_fixture_pipeline.ps1 --labels full --jobs 3 --json`).
- Positioning lane recently expanded with static-position fallback and explicit-size overflow coverage.
