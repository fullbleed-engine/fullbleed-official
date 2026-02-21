# CSS Broad Coverage Sprint (S14)

Source taxonomy: https://developer.mozilla.org/en-US/docs/Web/CSS

## Sprint Charter

- Sprint ID: `S14_BROAD_COVERAGE_PUSH`
- Window: `2 weeks`
- Goal: maximize MDN module breadth and close baseline stage gaps before pursuing deep edge-case fidelity.
- Guiding policy: `breadth-first`. No deep edge-case work unless it unblocks a not-started module or a stage-wide blocker.

## Baseline (2026-02-21)

- Modules tracked: `22`
- `in_progress`: `22`
- `not_started`: `0`
- Full fixture lane: `85/85` passing

## Sprint Targets

- Move `not_started` modules from `3` to `0` (all modules at least `in_progress`).
- Raise module progress from `86.36%` to `100%` progressed (no module left `not_started`).
- Raise paint stage progress from `70.00%` to at least `80%`.
- Add `>= 12` new broad-coverage fixtures (module unlock + baseline behavior fixtures).
- Keep deterministic lane green (`full` fixture suite always passing on merge points).
- Add explicit layout execution policy controls (`eager` vs `lazy`) with required user opt-in for lazy performance cost.

## Backlog Discipline

- Capacity split: `80%` broad coverage, `20%` edge/regression.
- WIP limits:
- Maximum `2` concurrent module workstreams.
- Maximum `1` deep edge-case item in flight at any time.
- Acceptance policy:
- Every completed task must add or update at least one fixture with compute/layout/paint checks.
- Any non-trivial unsupported behavior must emit deterministic diagnostics and land with fixture coverage.

## Priority Backlog (Broad Coverage First)

| Priority | MDN Module | Current | Sprint Target | Primary Outcome |
|---|---|---|---|---|
| P0 | Filters/effects/compositing | not_started | in_progress | deterministic parser + fallback/diagnostics path + baseline fixture coverage |
| P0 | Multi-column | not_started | in_progress | deterministic single-column fallback policy + parser/compute flags + fixture coverage |
| P0 | Writing modes and logical properties | not_started | in_progress | logical-to-physical mapping baseline for horizontal-tb + fixture coverage |
| P1 | Grid | partial | broader partial | minimum dedicated grid track path (reduce grid-as-flex reliance) + baseline fixtures |
| P1 | Paged media | partial | broader partial | broaden @page + break control coverage with deterministic pagination assertions |
| P1 | Fragmentation | partial | broader partial | consistent split behavior across block/flex/table on core break scenarios |
| P1 | Backgrounds and borders | partial | broader partial | broaden paint behavior matrix with stable draw-order fixtures |
| P1 | Overflow | partial | broader partial | visible/hidden + baseline clipping interactions with positioned/transform content |
| P2 | Positioning edge cases | partial | park unless blocker | queue edge cases after P0/P1 breadth gates are met |

## Breadcrumbed Sprint Tasks

- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_FILTERS > Define deterministic supported subset and known-loss diagnostics`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_FILTERS > Land baseline fixtures for filter/compositing fallback behavior`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_MULTICOL > Implement deterministic multicol fallback contract (single-column degrade)`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_MULTICOL > Add parser/compute/layout fixtures validating fallback + diagnostics`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_LOGICAL > Implement horizontal-tb logical property mapping baseline`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_LOGICAL > Add fixtures for logical margin/padding/inset mapping`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_GRID_BASELINE > Add dedicated grid baseline path and fixture coverage`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAGED_FRAGMENTATION_BREADTH > Expand @page/break coverage across block/flex/table`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_BREADTH > Expand overflow/background paint-order fixture matrix`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_VALIDATION_AND_DETERMINISM > Promote new fixtures to full lane with stability hashes`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_ENGINE_POLICY > Add layout_strategy contract with explicit lazy cost acceptance and deterministic convergence diagnostics`

## Latest Execution Update (2026-02-21)

- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_PARITY > Harden box-shadow blur/stack rendering` in `src/flowable.rs` with weighted multi-pass blur and negative spread support.
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_PARITY > Lock negative spread behavior with fixture coverage` via `_css_working/fixtures/box_shadow_negative_spread_offset_paints_shrunk_shadow.json`.
- Validation refresh: full fixture lane now `85/85` passing and parity status check remains green.

- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_LOGICAL > Implement horizontal-tb logical property mapping baseline` in `src/style.rs` for logical size/margin/padding/inset longhands and shorthands across typed + unparsed declaration paths.
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_LOGICAL > Add fixtures for logical margin/padding/inset mapping` via:
- `_css_working/fixtures/logical_margin_padding_horizontal_tb_ltr.json`
- `_css_working/fixtures/logical_inset_horizontal_tb_ltr.json`
- `_css_working/fixtures/logical_inline_block_size_min_max.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_MULTICOL > Implement deterministic multicol fallback contract (single-column degrade)` with explicit `MULTICOL_SINGLE_COLUMN_FALLBACK` known-loss diagnostics in `src/style.rs`.
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_MULTICOL > Add parser/compute/layout fixtures validating fallback + diagnostics` via:
- `_css_working/fixtures/diagnostics_multicol_single_column_fallback.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_FILTERS > Define deterministic supported subset and known-loss diagnostics` with explicit `FILTERS_EFFECTS_FALLBACK` logging for unsupported effects/compositing declarations.
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_FILTERS > Land baseline fixtures for filter/compositing fallback behavior` via:
- `_css_working/fixtures/diagnostics_filters_effects_fallback.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_GRID_BASELINE > Add dedicated grid baseline path and fixture coverage` in:
- `src/style.rs` for `grid-template-rows` plus baseline `grid-column/grid-row/grid-area` start-line parsing into computed style.
- `src/html.rs` for deterministic slot-based grid item ordering with placeholder-cell insertion to preserve explicit row/column placement in the current equal-track baseline path.
- `_css_working/fixtures/grid_template_rows_columns_matrix.json`
- `_css_working/fixtures/grid_explicit_row_column_starts.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_GRID_BASELINE > Tighten deterministic grid placement behavior` in:
- `src/html.rs` for explicit-slot collision fallback (later conflicting items move to next free slot), auto-row/auto-column completion when one axis is explicit, and deterministic column derivation from `grid-template-rows` when columns are omitted.
- `src/style.rs` for broader `grid-template-columns` track counting over `repeat(...)` segments in mixed track lists.
- `_css_working/fixtures/grid_rows_only_column_derivation.json`
- `_css_working/fixtures/grid_conflicting_explicit_slots_fall_forward.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAGED_FRAGMENTATION_BREADTH > Expand @page/break coverage across block/flex/table` via:
- `_css_working/fixtures/paged_break_before_forced_block.json`
- `_css_working/fixtures/fragmentation_break_inside_avoid_block.json`
- `_css_working/fixtures/table_header_repeat_across_pages.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_BREADTH > Expand overflow/background paint-order fixture matrix` via:
- `_css_working/fixtures/overflow_hidden_clips_absolute_child.json`
- `_css_working/fixtures/overflow_visible_allows_absolute_bleed.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_BREADTH > Expand border paint-order matrix and evaluator-side border color mapping` in:
- `src/style.rs` for side-specific border color parse/compute channels (`border-top/right/bottom/left-color` + logical side aliases) with deterministic var-resolution support.
- `src/flowable.rs` and `src/html.rs` for per-edge container border paint colors (not just uniform border-color fallback), including side-override rendering in the deterministic paint path.
- `_css_working/fixtures/borders_uniform_frame_paint_matrix.json`
- `_css_working/fixtures/borders_left_color_override.json`
- Completed: `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PEC_BREADTH > Canonical parser-evaluator-calculator path for custom/unparsed length math` in:
- `src/style.rs` to route `length_spec_from_custom_tokens` through canonical `LengthPercentage::parse_string(...)` before heuristic fallback, and to extend deterministic evaluator reduction for `min()/max()/clamp()/abs()` over context-free comparable `CalcLength` domains.
- `_css_working/fixtures/custom_property_math_min_max_clamp_lengths.json` for custom-property `min/max/clamp` width resolution through compute/layout/paint.
- Validation: targeted paged/fragmentation lane passes (`3/3`), targeted paint/overflow lane passes (`2/2`), targeted border paint lane passes (`2/2`), targeted values/math lane passes (`2/2`), full fixture lane (`--labels full`) passes (`85/85`), and parity status check is green (`python tools/generate_css_parity_status.py --check --json`).
- Deferred (non-blocking backlog): `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_MODULE_UNLOCK_LOGICAL > logical min/max constraint semantics + writing-mode/direction axis remapping beyond horizontal-tb/LTR`.

## Iteration Gap Intake (2026-02-20)

Validated major gaps from visual-iteration loop and folded into sprint backlog:

- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_BUILD_PATH_DISCIPLINE > Enforce source-sync runtime before visual diagnosis`
  - Symptom: stale extension build produced false negatives (empty fill paint misses, chart element drops).
  - Action: require venv + `maturin develop --release --features python` before parity diagnosis runs.
  - Status: `completed`.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PEC_BREADTH > Resolve calc(var(--x) * scalar) in pending length path`
  - Symptom: pending size resolver fell through to `auto` for `calc(var(--pct) * 1%)`.
  - Action: add parser-evaluator-calculator support in pending var length resolution and lock with unit + fixture regression.
  - Status: `completed`.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PEC_BREADTH > Extend calc(var(...)) additive mixed-unit resolution`
  - Symptom: var-driven additive expressions (`calc(var(--a) + 12pt)`, `calc(100% - var(--pad))`) were not first-class in pending resolver path.
  - Action: add deterministic additive evaluation over canonical `CalcLength` composition, including scaled-term combinations.
  - Status: `completed`.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_BREADTH > Verify empty-element fill painting on latest source build`
  - Symptom: empty chart fill blocks appeared non-painting in stale runtime.
  - Action: revalidate on latest build path; keep fixture coverage for empty block backgrounds.
  - Status: `completed` (latest-source behavior paints correctly).
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > Conic-gradient support and deterministic fallback diagnostics`
  - Symptom: conic-gradient-driven gauges degrade on current paint path.
  - Action: implement first-class conic-gradient parser + painter for explicit color-stop forms; keep deterministic fallback diagnostics for unsupported stop expressions.
  - Status: `completed` (explicit conic paint landed in `src/style.rs` + `src/flowable.rs`; fallback path retained for `currentColor/calc(var(...))` stop syntax).
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > Conic-gradient dynamic stop expressions`
  - Symptom: `conic-gradient(... currentColor calc(var(--pct) * 1%) ...)` remains fallback in current canonical path.
  - Action: add computed-stage conic stop evaluation over resolved custom properties + currentColor binding.
  - Status: `completed`.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_COLOR_FUNCTIONS > color-mix compatibility hardening`
  - Symptom: iteration flagged parser risk; latest-source probes show baseline support, edge cases still unpinned.
  - Action: add explicit compatibility fixture matrix to close ambiguity.
  - Status: `queued`.

Latest landed artifacts from this intake:

- `_css_working/fixtures/diagnostics_conic_gradient_fallback_with_color.json`
- `_css_working/fixtures/diagnostics_conic_gradient_fallback_no_color.json`
- `_css_working/fixtures/background_conic_gradient_supported_quadrants.json`
- `_css_working/fixtures/custom_property_calc_var_additive_mixed_lengths.json`

## Execution Plan

| Week | Focus | Expected Output |
|---|---|---|
| Week 1 | P0 module unlocks | 3 previously not-started modules moved to in_progress with fixtures |
| Week 2 (first half) | P1 breadth uplift | grid/paged/fragmentation/paint baseline fixtures added |
| Week 2 (second half) | hardening and gate | full lane stable, parity status regenerated, deferred edge backlog captured |

## Validation Protocol (Required)

- Run targeted fixture lane for each workstream before merge.
- Emit artifacts (`output.pdf`, `output_page*.png`, `render_result.json`, `hashes.json`) for every new fixture.
- For any lazy-layout run, record `jit.layout_strategy` diagnostics (passes, convergence, budget hit) and treat non-convergence as backlog candidate unless blocking.
- Run full lane at integration points:
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels full --jobs 3 --json`
- Run parity status check after sprint changes:
- `python tools/generate_css_parity_status.py --check --json`

## Exit Gates

- Gate 1: `not_started` modules = `0`.
- Gate 2: full fixture lane passes at sprint end.
- Gate 3: parity status artifact regenerated and committed.
- Gate 4: explicit deferred edge-case backlog documented for next sprint.

## Full Re-Eval Snapshot (2026-02-21)

Validation executed from latest source build path:

- `cargo test -- --nocapture`: `276/276` passing.
- `cargo check`: passed.
- `maturin develop` (project venv): passed.
- `python tools/run_css_fixture_suite.py --labels full --jobs 4 --out _css_working/tmp/fixture_full_latest.json`: `85/85` passing.
- `python tools/generate_css_parity_status.py --check --json`: passed.
- `FULLBLEED_DEBUG=1 FULLBLEED_PERF=1 FULLBLEED_EMIT_PNG=0 python examples/css_parity_canonical_100/run_example.py`: completed.
- `FULLBLEED_DEBUG=1 FULLBLEED_PERF=1 FULLBLEED_FANCY=1 FULLBLEED_EMIT_PNG=1 python examples/css_visual_charts_showcase/run_example.py`: completed.
- `WS_PAINT_PARITY` checkpoint: box-shadow hardened in `src/flowable.rs` with weighted multi-pass blur approximation and negative spread support (no more stepped halo artifacts in fancy output).

Performance snapshot:

- Canonical 100-page benchmark: `layout.strategy=216730.735ms`, `story=80090.484ms`, `layout=135226.583ms`, `pdf.link=54.189ms`, `pages=100`, `commands=251251`, `converged=1`, `budget_hit=0`.
- Visual charts benchmark: `layout.strategy=2323.450ms`, `story=1668.749ms`, `layout=617.102ms`, `pdf.link=25.324ms`, `pages=3`, `commands=10454`, `converged=1`, `budget_hit=0`.

Known-loss and unresolved snapshot:

- Canonical benchmark known-loss: `PAGE_SIZE_OVERRIDDEN:1`.
- Visual charts known-loss: `PAGE_SIZE_OVERRIDDEN:1` only.
- Visual charts unresolved expressions: none observed in latest debug run (`vars_unresolved` arrays empty).

Conic-gradient fixture alignment update:

- Converted stale fallback fixtures to malformed conic syntax so diagnostics coverage remains valid:
- `_css_working/fixtures/diagnostics_conic_gradient_fallback_no_color.json`
- `_css_working/fixtures/diagnostics_conic_gradient_fallback_with_color.json`
- Supported conic path remains validated via:
- `_css_working/fixtures/background_conic_gradient_supported_quadrants.json`

## Final Parity Push Backlog (Breadth First)

- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_FUNCTION_BREADTH > Expand filter/backdrop-filter function coverage beyond current subset (blur/saturate lanes) with deterministic diagnostics for unsupported functions.`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_CLIP_PATH_BREADTH > Expand clip-path support beyond inset(...) to circle/ellipse/polygon subsets with fixture-backed fallback diagnostics for unsupported path grammar.`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_BLEND_BREADTH > Expand mix-blend-mode coverage beyond normal/multiply/screen and harden isolation semantics across nested containers.`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_BOX_SHADOW_BREADTH > Add multi-shadow list + inset blur semantics (current path supports single shadow with improved weighted blur and negative spread).`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_TABLE_LAYOUT_GOLD > Gold-plate table layout parity path (`table-layout:auto/fixed`, header behavior, deterministic width pressure handling) with expanded fixture matrix.`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_BACKGROUND_STACKS > Add multi-layer background image compositing semantics and fixture matrix for stacked gradients/images.`
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_COLOR_FUNCTIONS > Land explicit color-mix compatibility fixture matrix (edge syntax + interpolation space cases).`

Release posture from this snapshot:

- Broad deterministic lane is healthy (`85/85` fixtures, stable diagnostics, no open module in `not_started`).
- Full CSS3 near-parity is not yet achieved due effects/compositing + remaining chart-class value/paint gaps.
- Recommended track: ship as `parity RC (broad coverage)` with explicit compatibility contract, then execute backlog above for `final parity push`.

## Deferred Edge-Case Queue Policy

- Edge-case items discovered during S14 are logged but deferred unless they block a P0/P1 breadth task.
- Deferred items must include:
- breadcrumb
- impacted module
- minimal reproduction fixture id
- blocker rationale (`blocking` or `non-blocking`)
