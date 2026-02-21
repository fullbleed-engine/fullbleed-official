# CSS Coverage and Parity Status

This document is the canonical statement of validated CSS coverage for Fullbleed's deterministic HTML/CSS-to-PDF engine.

Scope baseline: static, paged document rendering (transactional/report workflows), not a live browser runtime.

Source taxonomy: https://developer.mozilla.org/en-US/docs/Web/CSS

## Validation Basis

As of February 21, 2026:

- Fixture suite: `85/85` passing (`_css_working/tmp/fixture_full_latest.json`)
- Parity status check: green (`python tools/generate_css_parity_status.py --check --json`)
- Canonical benchmark lane: `examples/css_parity_canonical_100`
- Visual stress lane: `examples/css_visual_charts_showcase`

Primary generated artifacts:

- `_css_working/css_parity_status.json`
- `_css_working/css_broad_coverage_sprint_s14.md`
- `_css_working/tmp/fixture_full_latest.json`

## Current Coverage Summary

Tracked CSS modules: `22`

- Module status: `22/22 in_progress`
- Parser stage: `22/22 partial`
- Compute stage: `22/22 partial`
- Layout stage: `16/22 partial`, `6/22 n/a`
- Paint stage: `10/22 partial`, `12/22 n/a`

## Module Matrix (22 Tracked)

Legend: `p` parser, `c` compute, `l` layout, `pa` paint.

| Module | Stage footprint | Validated now | Backlog focus |
| --- | --- | --- | --- |
| Syntax and at-rules | p:`partial` c:`partial` l:`n/a` pa:`n/a` | Typed + unparsed declaration paths in place | Advanced at-rule breadth and edge grammar |
| Selectors | p:`partial` c:`partial` l:`n/a` pa:`n/a` | Deterministic selector parser/indexing baseline | Broader selector/pseudo-class parity |
| Cascade and inheritance | p:`partial` c:`partial` l:`n/a` pa:`n/a` | Specificity + source order + `!important` channels | Layer/revert/revert-layer breadth |
| Values and units | p:`partial` c:`partial` l:`n/a` pa:`n/a` | Canonical calc path with var-chain, `min/max/clamp` subsets | Wider value grammar/function breadth |
| Box model | p:`partial` c:`partial` l:`partial` pa:`partial` | Margin/padding/border/box-sizing baseline | Edge-case constraint and interaction hardening |
| Display and formatting contexts | p:`partial` c:`partial` l:`partial` pa:`n/a` | Block/inline/flex/table/grid-like lowering baseline | Broader formatting-context parity |
| Positioning | p:`partial` c:`partial` l:`partial` pa:`n/a` | Relative/absolute/fixed deterministic baseline | Sticky/edge semantic breadth and interactions |
| Sizing | p:`partial` c:`partial` l:`partial` pa:`n/a` | Width/height/min/max core coverage | Intrinsic sizing and pressure edge cases |
| Text and fonts | p:`partial` c:`partial` l:`partial` pa:`partial` | Text styling, fallback, shaping hooks baseline | Typographic edge behavior breadth |
| Backgrounds and borders | p:`partial` c:`partial` l:`partial` pa:`partial` | Solid + gradient backgrounds, border color propagation | Multi-layer backgrounds and remaining border effects |
| Lists and counters | p:`partial` c:`partial` l:`partial` pa:`partial` | List rendering baseline | Counter/generated-content breadth |
| Overflow | p:`partial` c:`partial` l:`partial` pa:`partial` | `visible`/`hidden` clipping and bleed fixtures | Additional overflow modes and clip semantics |
| Flexbox | p:`partial` c:`partial` l:`partial` pa:`n/a` | Core flex flow/alignment subsets | Spec edge cases and distribution pressure |
| Grid | p:`partial` c:`partial` l:`partial` pa:`n/a` | Deterministic baseline placement and repeat counting | Dedicated solver breadth (autoplacement/track sizing/span) |
| Tables | p:`partial` c:`partial` l:`partial` pa:`partial` | Table baseline + header repeat across pages | `table-layout:auto/fixed` edge hardening |
| Transforms and coordinate spaces | p:`partial` c:`partial` l:`n/a` pa:`partial` | 2D transforms + transform-origin + composition | 3D/perspective breadth |
| Filters/effects/compositing | p:`partial` c:`partial` l:`partial` pa:`partial` | Effects subset (`filter` saturate, `backdrop-filter` blur/saturate, blend subset, clip-path inset) | Function breadth, clip-path shapes, blend/isolation breadth, multi-shadow |
| Multi-column | p:`partial` c:`partial` l:`partial` pa:`partial` | Deterministic single-column fallback contract | True multicol balancing/span/rule semantics |
| Fragmentation | p:`partial` c:`partial` l:`partial` pa:`n/a` | Core break controls across block/flex/table subsets | Deeper fragmentation edge behavior |
| Paged media | p:`partial` c:`partial` l:`partial` pa:`n/a` | Core `@page` + break control subsets | Named pages/margin-box breadth |
| Writing modes and logical properties | p:`partial` c:`partial` l:`partial` pa:`partial` | Horizontal-tb/LTR logical mapping baseline | Axis remap beyond horizontal-tb/LTR |
| Custom properties and API-adjacent parsing | p:`partial` c:`partial` l:`n/a` pa:`n/a` | Deterministic custom-property resolution baseline | Broader API-adjacent grammar/interop lanes |

Interpretation:

- Fullbleed has broad, validated baseline coverage across all tracked modules.
- "Partial" means implemented, fixture-validated subsets with deterministic fallback/diagnostic policy for unsupported forms.
- Near-parity for static reporting is practical; full browser-level CSS parity is not yet claimed.

## Validated Feature Areas

Validated means parser -> evaluator -> calculator -> layout/paint is exercised by tests/fixtures in the current lane.

- Cascade and specificity ordering with typed + unparsed declaration paths
- Custom properties and var-chain resolution with fallback and cycle-safe behavior
- Length math including `calc()`, `min()`, `max()`, `clamp()`, additive mixed-unit paths
- Box model and border propagation, including side-specific border color paint behavior
- Positioning baseline (`relative`, `absolute`, `fixed`) with deterministic containing-block behavior
- Overflow baseline (`visible` and `hidden`) with clipping/bleed fixtures
- Flex baseline including wrapped-line/content alignment subsets
- Grid baseline including explicit row/column start placement, repeat track counting, deterministic slot fallback
- Table baseline including split behavior and header repeat coverage across pages
- Paged-media fragmentation baseline (`break-before`, `break-inside` core paths)
- 2D transforms (`translate`, `scale`, `rotate`, `skew`, `matrix`) with transform-origin and composition model
- Gradient paint baseline including `linear-gradient`, `radial-gradient`, and conic gradient support paths
- Effects subset with deterministic behavior in current lane: `filter: saturate(...)`
- Effects subset with deterministic behavior in current lane: `backdrop-filter: blur(...) saturate(...)`
- Effects subset with deterministic behavior in current lane: `mix-blend-mode: normal | multiply | screen`
- Effects subset with deterministic behavior in current lane: `clip-path: inset(...)`
- Box-shadow baseline with weighted blur hardening and negative spread support

## Known Gaps (Active Backlog)

These are known, tracked gaps for final parity push:

- Broader `filter` and `backdrop-filter` function coverage beyond current subset
- `clip-path` shapes beyond `inset(...)` (`circle`, `ellipse`, `polygon`, etc.)
- Additional blend modes and isolation semantics breadth
- Multi-shadow list semantics and full inset blur parity
- Multi-layer background image compositing semantics
- Table layout edge semantics hardening (`table-layout:auto/fixed` pressure edges)
- Writing-mode/direction remapping breadth beyond current horizontal-tb/LTR baseline
- Remaining color-function edge compatibility matrix (`color-mix` hardening)

## Iteration Progress Snapshot (S14)

Resolved during this sprint's visual-iteration loop:

- Build-path drift removed (`venv` + `maturin develop --release --features python` discipline) so parity diagnosis runs against latest source.
- Parser -> evaluator -> calculator path hardened for pending var-length math (`calc(var(--x) * scalar)` and additive mixed-unit forms).
- Empty-element fill paint path revalidated on latest build (no stale-runtime false negatives).
- Conic gradients moved from fallback-only behavior to first-class parser + painter support (with deterministic diagnostics retained for unsupported forms).
- Border/overflow/grid/paged-media baseline fixture matrix expanded and green in full lane (`85/85`).

Still open for final parity push:

- Effects breadth beyond current subset (`filter`/`backdrop-filter`, blend/isolation breadth).
- Clip-path shape breadth beyond `inset(...)`.
- Multi-shadow list semantics and inset-blur fidelity.
- Multi-layer background compositing semantics.
- Table-layout pressure and edge semantics hardening.
- Additional `color-mix(...)` edge-case compatibility matrix coverage.

## Deterministic Compatibility Contract

When CSS is not supported, behavior must be:

- Deterministic (no random/non-reproducible geometry or paint drift)
- Diagnosable (structured known-loss/fallback signals)
- Fixture-covered (repro + regression assertions)

Recent visual stress runs show the effects fallback lane is materially reduced; the primary known-loss signal observed in the current showcase debug output is `PAGE_SIZE_OVERRIDDEN` for explicit runtime page size precedence.

## How To Re-Validate

Run from repo root with the project venv:

```powershell
.\.venv\Scripts\python tools\run_css_fixture_suite.py --labels full --jobs 4 --out _css_working\tmp\fixture_full_latest.json
.\.venv\Scripts\python tools\generate_css_parity_status.py --check --json
```

Optional visual stress lane:

```powershell
$env:FULLBLEED_DEBUG='1'
$env:FULLBLEED_PERF='1'
$env:FULLBLEED_FANCY='1'
$env:FULLBLEED_EMIT_PNG='1'
.\.venv\Scripts\python examples\css_visual_charts_showcase\run_example.py
```

## Notes on Bootstrap

Bootstrap remains a vendored asset baseline for scaffolded workflows, but Bootstrap preflight screenshots are no longer treated as the canonical coverage definition. Canonical CSS coverage claims are now derived from parity fixtures and status artifacts in `_css_working/`.
