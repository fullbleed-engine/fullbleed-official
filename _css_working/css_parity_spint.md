# CSS Parity Sprint Plan (Deterministic Fix-Point Renderer)

Source taxonomy: https://developer.mozilla.org/en-US/docs/Web/CSS

## Mission

Bring the Rust core to practical "all CSS" parity for static, paged HTML/CSS-to-PDF rendering while preserving deterministic outputs (bit-for-bit stable with pinned inputs).

## Scope Contract

- `In scope`: Parse, cascade, compute, layout, and paint behavior for CSS used in static document rendering.
- `In scope`: Print/paged media behavior, fragmentation, counters, generated content, and typography fidelity.
- `Out of scope` for v1 parity target: browser-only runtime features requiring a live event loop (`:hover` dynamics, DOM mutation driven transitions, timeline animations as real-time playback).
- `Policy`: Unsupported runtime-only features must degrade deterministically and emit structured diagnostics.

## Breadcrumb Model

Breadcrumb format:

`CSS_PARITY > Phase > Sprint > Workstream > Task`

Example:

`CSS_PARITY > P2_COMPUTED_VALUES > S04_UNITS_AND_CALC > WS_VALUE_SOLVER > Implement percentage resolution against containing block`

## Definition Of Done (Global)

- Deterministic render hash stable across repeated runs and CI agents with pinned assets.
- CSS compatibility matrix generated and versioned.
- Every implemented module has:
  - parser tests
  - computed-value tests
  - layout/paint golden tests
  - regression fixtures for previously found bugs
- Unsupported behavior emits `machine-readable` diagnostics with actionable fallback notes.

## Determinism Guardrails

- Fixed-point arithmetic for layout-critical math (`no float drift` in geometry decisions).
- Canonical cascade ordering for equal-specificity/equal-origin ties.
- Stable selector matching order and stable iteration order on hash-backed collections.
- Bounded fix-point loops with explicit convergence checks and deterministic bailout.
- Stable font fallback selection and shaping option ordering.
- Stable image decode and color conversion paths.

## Validation Paradigm (Formalized)

Validation model:

- `V0 Unit`: parser/value/cascade function tests.
- `V1 Stage`: `compute_style`, `wrap/split/draw`, page-template/pagination behavior.
- `V2 Visual`: PNG output comparison and pixel-level assertions.
- `V3 Artifact Determinism`: PDF + PNG hash equality across reruns/thread-count changes.
- `V4 Scenario`: end-to-end fixtures (scaffolded examples + template compose workflows).

Authoritative validation loop per CSS change:

1. Run focused stage tests for touched subsystem.
2. Render targeted fixture pages to PNG using engine image pipeline.
3. Compare PNGs against expected visual baseline.
4. Run deterministic hash checks (repeat render, thread variance).
5. Promote to golden-suite verify before merge.

Required artifact contract per fixture:

- `render_result.json`:
  - parser diagnostics
  - computed-style snapshot (selected nodes/properties)
  - pagination events (`break_before/after/inside`, split points)
  - known-loss diagnostics (if any)
- `output.pdf`
- `output_page*.png`
- `hashes.json`:
  - `pdf_sha256`
  - `image_sha256[]`
  - `artifact_sha256` (normalized set hash)

Fixture metadata contract (`_css_working/fixtures/*.json`):

```json
{
  "id": "fixture_id",
  "required_features": ["css-feature-a", "css-feature-b"],
  "expected_warnings": [],
  "html": "<!doctype html>...",
  "css": "@page { ... } ...",
  "expected": {
    "page_count": 1,
    "compute_assertions": [
      {
        "node_contains": "section#target",
        "style": {
          "display": "Block",
          "background": "#00ff00"
        },
        "vars_unresolved_count": 0
      }
    ],
    "layout_assertions": [
      {
        "kind": "color_run_length",
        "page": 1,
        "axis": "x",
        "fixed": 200,
        "start": 0,
        "rgba": [0, 255, 0, 255],
        "tolerance": 8,
        "expected_length": 480,
        "length_tolerance": 1
      }
    ],
    "paint_samples": [
      {
        "page": 1,
        "x": 20,
        "y": 20,
        "rgba": [0, 0, 255, 255],
        "tolerance": 8
      }
    ]
  },
  "stability_hash": "artifact_hash_or_null"
}
```

Repository-native commands (standard lane):

```powershell
python goldens/run_golden_suite.py verify
python examples/template-flagging-smoke/run_cli_determinism_smoke.py
python examples/template-flagging-smoke/run_cli_compose_image_smoke.py
```

PNG iteration lane (developer fast loop):

- Use engine/CLI image emission (`render_image_pages` or CLI `--emit-image`) on narrowed fixtures.
- Validate:
  - geometry deltas (layout/flow regressions)
  - color/paint order deltas
  - template-compose background presence for composed mode (`image_mode=composed_pdf`)
- If intended visual change occurs, regenerate only affected goldens with rationale in PR notes.

Debug escalation lane (verbose, exception-only):

- Enable engine debug only when normal validation cannot localize a defect.
- Use:
  - `debug=true`
  - `debug_out='filename.log'`
- Expected output includes highly verbose style parsing and object draw traces.
- Policy: do not run this mode as part of routine CI/fast-loop iteration; attach/redact only relevant excerpts when filing/parsing an incident.

Determinism policy:

- Every promoted fixture must pass:
  - same-input rerun hash equality
  - multi-thread parity where applicable (Rayon thread-count variance)
  - stable JSON contract fields for diagnostics payloads
- Any nondeterministic result is a release blocker for the sprint owning that module.

Defect triage classes:

- `P0 Determinism`: hash mismatch or non-reproducible pagination.
- `P1 Visual Regression`: unintended PNG delta.
- `P2 Conformance Gap`: spec mismatch with stable output.
- `P3 Diagnostic Gap`: unsupported behavior lacks actionable signal.

Sprint exit gate augmentation:

- No sprint closes without:
  - `V2` green on scoped fixtures
  - `V3` green on scoped fixtures
  - golden-suite verify green for impacted domains
  - updated parity ledger entries for each implemented property.

## Program Cadence

- Sprint length: 2 weeks
- Total initial program: 14 sprints
- Milestone reviews: every 2 sprints
- Ship policy: no milestone closes with unresolved determinism regressions

## Execution Commands (P0)

Baseline build checkpoint:

```powershell
cargo check -q
```

Parity status generation:

```powershell
python tools/generate_css_parity_status.py --json
python tools/generate_css_parity_status.py --check --json
```

CI artifact generation equivalent:

```powershell
python tools/generate_css_parity_status.py --out output/css_parity_status.ci.json --json
```

Fixture harness execution (S01 scaffold):

```powershell
python tools/run_css_fixture_suite.py --json
```

Pin/update fixture stability hashes intentionally:

```powershell
python tools/run_css_fixture_suite.py --update-stability --json
```

## Sprint Execution Status (2026-02-21)

Active focus reset:

- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PROGRAM_ALIGNMENT > Broad-coverage-first sprint formalized` in `_css_working/css_broad_coverage_sprint_s14.md` with module unlock targets, WIP limits, validation gates, and edge-case deferral policy.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_GAP_INTAKE > Visual iteration gap inventory formalized` with explicit backlog entries for stale build-path diagnosis, `calc(var(...))` evaluator gaps, empty-fill validation, conic-gradient fallback, and `color-mix` hardening.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PEC_BREADTH > Pending-length resolver coverage for calc(var(--x) * scalar)` prioritized as the active parser -> evaluator -> calculator parity lane.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > Conic-gradient parser + painter baseline` completed in `src/style.rs` and `src/flowable.rs` for explicit conic color-stop forms.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > Conic-gradient fallback diagnostics hardening` completed with dynamic stop expressions (including `currentColor` + `calc(var(...))`) resolved without fallback diagnostics.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PEC_BREADTH > Pending-length resolver additive mixed-unit support` completed for `calc(var(...)+/-...)` forms in the var-resolution path.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > clip-path inset evaluator + painter path` completed with `clip-path: inset(...)` resolved into deterministic clip rects in `ContainerFlowable`.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > radial-gradient parser + painter baseline` completed in `src/style.rs` + `src/flowable.rs`, including var-aware gradient-stop resolution and unitless-zero position normalization (`10% 0`).
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_EFFECTS_BACKLOG > Backdrop-filter + blend subset path` completed (`backdrop-filter: blur/saturate`, `mix-blend-mode: normal/multiply/screen`, `filter: saturate`) with no `FILTERS_EFFECTS_FALLBACK` diagnostics in latest fancy run.
- `CSS_PARITY > P7_LAST_MILE > S14_BROAD_COVERAGE_PUSH > WS_PAINT_PARITY > Box-shadow blur hardening` completed with weighted blur passes and negative spread support in `src/flowable.rs`.
- Remaining breadth gaps (queued): full filter function space, clip-path shapes beyond inset, blend-mode breadth beyond current subset, multi-shadow list semantics, and multi-layer background compositing.

Completed breadcrumbs:

- `CSS_PARITY > P0_BASELINE > S00_PROGRAM_SETUP > WS_PARITY_LEDGER > Generate versioned parity status artifact` implemented with `tools/generate_css_parity_status.py`, `_css_working/css_parity_ledger.json`, `_css_working/css_parity_status.json`.
- `CSS_PARITY > P0_BASELINE > S00_PROGRAM_SETUP > WS_CI_WIRING > Publish parity status in CI` implemented in `.github/workflows/ci.yml` with `--check`, CI artifact emission, and upload.
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_FIXTURE_RUNNER > Stage-oriented fixture execution` implemented in `tools/run_css_fixture_suite.py` with parser/compute/layout/paint assertions and stability hash checks.
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_FIXTURE_METADATA > Fixture labels and deterministic hash lifecycle` implemented with `labels`, `--jobs`, `--update-stability`, and fixture metadata contract usage.
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_ARTIFACT_EMISSION > Persist inspectable fixture artifacts` implemented in `tools/run_css_fixture_suite.py` via `--emit-artifacts-dir`, emitting per-fixture `output.pdf`, `output_page*.png`, `hashes.json`, and `render_result.json`.
- `CSS_PARITY > P1_CASCADE_CORE > S02_SYNTAX_AND_AT_RULES > WS_DIAGNOSTICS > Unsupported/known-loss validation lane` implemented with `debug=true` + `debug_out` parsing and `expected_diagnostics` assertions.
- `CSS_PARITY > P1_CASCADE_CORE > S03_SELECTORS_AND_SPECIFICITY > WS_REGRESSION > Background shorthand color recovery` implemented in `src/style.rs` (named-color parsing fallback, shorthand token extraction, and direct shorthand color field handling) with added style unit tests.
- `CSS_PARITY > P1_CASCADE_CORE > S03_SELECTORS_AND_SPECIFICITY > WS_PARSER_HARDENING > Top-level combinator parsing for complex selector tokens` implemented in `src/style.rs` by scoping combinator parsing to top-level selector context (outside `[]`, `()`, and quoted segments).
- `CSS_PARITY > P1_CASCADE_CORE > S03_SELECTORS_AND_SPECIFICITY > WS_SELECTOR_FIXTURES > Add selector conformance fixtures` implemented with:
  - `_css_working/fixtures/selectors_attribute_variants.json`
  - `_css_working/fixtures/selectors_sibling_combinators.json`
  - `_css_working/fixtures/selectors_structural_pseudos.json`
- `CSS_PARITY > P1_CASCADE_CORE > S03_SELECTORS_AND_SPECIFICITY > WS_UNIT_REGRESSION > Attribute includes operator guard` implemented with `style::tests::attribute_includes_selector_matches_space_separated_values`.
- `CSS_PARITY > P2_COMPUTED_VALUES > S04_UNITS_AND_CALC > WS_FIXTURE_EXPANSION > Add units/calc parity fixtures` implemented with:
  - `_css_working/fixtures/units_calc_width_resolution.json`
  - `_css_working/fixtures/units_min_max_clamp_widths.json`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CSS_WIDE_KEYWORDS > Add unset/revert fallback handling in unparsed-value paths` implemented in `src/style.rs` for size, color/background/border color, display, text overflow/decoration, spacing, and edge properties.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > CSS-wide keyword reset tests` implemented with:
  - `style::tests::width_unset_resets_to_auto`
  - `style::tests::background_color_unset_resets_to_initial_none`
  - `style::tests::display_unset_resets_to_initial_inline`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_FIXTURE_EXPANSION > Add unset reset scenario fixture` implemented with `_css_working/fixtures/inheritance_unset_noninherited_resets.json`.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CUSTOM_PROPERTIES > Deterministic length var chain/fallback resolver with cycle-safe bailout` implemented in `src/style.rs` by:
  - adding map-based length expression resolution for pending size vars and edge var application,
  - preserving raw `var(...)` expressions for custom property fallback semantics,
  - enforcing cross-type custom-property overwrite semantics to avoid stale typed caches.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > Length custom-property chain/fallback/cycle tests` implemented with:
  - `style::tests::custom_property_length_var_chain_resolves`
  - `style::tests::custom_property_length_var_fallback_resolves`
  - `style::tests::custom_property_length_var_cycle_bails_out_deterministically`
  - `style::tests::custom_property_redefinition_clears_stale_length_cache`
  - `style::tests::edge_length_var_chain_resolves_through_custom_refs`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CUSTOM_PROPERTIES > Font-family var chain/fallback resolver` implemented in `src/style.rs` by resolving `font-family: var(...)` against custom-property reference graphs with bounded cycle-safe fallback.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > Font-family custom-property var tests` implemented with:
  - `style::tests::font_family_var_chain_resolves_custom_stack`
  - `style::tests::font_family_var_fallback_resolves_when_missing`
  - `style::tests::font_family_var_cycle_uses_fallback`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CUSTOM_PROPERTIES > Min/max size var pending-resolution coverage` implemented in `src/style.rs` for `min-width`, `min-height`, and `max-height` with deterministic fallback/cycle-safe resolution.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CASCADE_ORDERING > Clear stale pending var placeholders on later concrete declarations` implemented in `src/style.rs` so later concrete size declarations in the same cascade lane cannot be overridden by earlier pending `var(...)` placeholders.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > Size var ordering and min/max coverage tests` implemented with:
  - `style::tests::concrete_size_overrides_prior_pending_var`
  - `style::tests::min_and_max_height_vars_resolve`
  - `style::tests::min_width_var_cycle_does_not_override_initial`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_FIXTURE_EXPANSION > Add length var chain/fallback scenario fixture` implemented with `_css_working/fixtures/custom_property_length_chain_fallback.json`.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CUSTOM_PROPERTIES > Pending var resolution for inset/flex-basis/gap` implemented in `src/style.rs` for `left/top/right/bottom`, `flex-basis`, and `gap` unparsed paths, apply stage, and pending resolver.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_CASCADE_ORDERING > Clear stale inset/flex-basis/gap pending var placeholders on concrete override` implemented in `src/style.rs` so later concrete declarations deterministically win.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > Inset/flex-basis/gap var and ordering tests` implemented with:
  - `style::tests::gap_var_resolves_from_custom_length`
  - `style::tests::flex_basis_var_fallback_resolves`
  - `style::tests::inset_var_resolves_from_custom_length`
  - `style::tests::concrete_inset_overrides_prior_pending_var`
  - `style::tests::concrete_gap_overrides_prior_pending_var`
  - `style::tests::concrete_flex_basis_overrides_prior_pending_var`
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_GAP_LONGHANDS > Row/column gap support and normalization` implemented in `src/style.rs` for `row-gap` and `column-gap` typed and unparsed paths, including `normal` normalization to deterministic zero gap.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_DIAGNOSTICS_ALIGNMENT > Remove row-gap/column-gap from parsed-no-effect diagnostics` implemented in `src/style.rs` so supported gap longhands no longer emit false known-loss diagnostics.
- `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES > WS_UNIT_REGRESSION > Gap longhand regression tests` implemented with:
  - `style::tests::row_gap_and_column_gap_map_to_engine_gap_last_wins`
  - `style::tests::row_gap_var_resolves_from_custom_length`
  - `style::tests::column_gap_normal_resets_to_zero`
  - `style::tests::concrete_row_gap_overrides_prior_pending_var`
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_COMPUTE_ASSERTIONS > Fixture-level computed-style evaluation checks` implemented in `tools/run_css_fixture_suite.py` via `expected.compute_assertions` (node matching + style/value + unresolved-var assertions), with debug capture enabled only when assertions/diagnostics are requested.
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_FIXTURE_EXPANSION > Add computed-style assertions to selector and custom-property fixtures` implemented in:
  - `_css_working/fixtures/selectors_specificity_id_wins.json`
  - `_css_working/fixtures/custom_property_length_chain_fallback.json`
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_FIXTURE_EXPANSION > Promote compute assertions across the remaining parity fixture corpus` implemented in:
  - `_css_working/fixtures/custom_property_fallback_chain.json`
  - `_css_working/fixtures/diagnostics_unsupported_media_and_no_effect.json`
  - `_css_working/fixtures/fragmentation_long_paragraph.json`
  - `_css_working/fixtures/inheritance_unset_noninherited_resets.json`
  - `_css_working/fixtures/selectors_attribute_variants.json`
  - `_css_working/fixtures/selectors_sibling_combinators.json`
  - `_css_working/fixtures/selectors_structural_pseudos.json`
  - `_css_working/fixtures/solid_red_block.json`
  - `_css_working/fixtures/two_page_break_colors.json`
  - `_css_working/fixtures/units_calc_width_resolution.json`
  - `_css_working/fixtures/units_min_max_clamp_widths.json`
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_LAYOUT_ASSERTIONS > Fixture-level geometry validation from rendered PNG output` implemented in `tools/run_css_fixture_suite.py` via `expected.layout_assertions` (`color_run_length` checks over rendered pages), and adopted in:
  - `_css_working/fixtures/custom_property_fallback_chain.json`
  - `_css_working/fixtures/custom_property_length_chain_fallback.json`
  - `_css_working/fixtures/diagnostics_unsupported_media_and_no_effect.json`
  - `_css_working/fixtures/flex_flow_row_wrap_layout.json`
  - `_css_working/fixtures/flex_place_items_container_alignment.json`
  - `_css_working/fixtures/flex_place_self_item_override.json`
  - `_css_working/fixtures/flex_two_column_distribution.json`
  - `_css_working/fixtures/fragmentation_long_paragraph.json`
  - `_css_working/fixtures/gap_row_column_var_resolution.json`
  - `_css_working/fixtures/inheritance_unset_noninherited_resets.json`
  - `_css_working/fixtures/inset_left_var_resolution.json`
  - `_css_working/fixtures/position_relative_offset_preserves_flow_slot.json`
  - `_css_working/fixtures/position_relative_right_bottom_offsets.json`
  - `_css_working/fixtures/position_relative_left_right_precedence.json`
  - `_css_working/fixtures/position_relative_top_bottom_precedence.json`
  - `_css_working/fixtures/position_relative_percent_left_containing_width.json`
  - `_css_working/fixtures/position_relative_calc_left_containing_width.json`
  - `_css_working/fixtures/position_relative_percent_top_containing_height.json`
  - `_css_working/fixtures/position_relative_calc_right_containing_width.json`
  - `_css_working/fixtures/position_relative_calc_bottom_containing_height.json`
  - `_css_working/fixtures/selectors_attribute_variants.json`
  - `_css_working/fixtures/selectors_sibling_combinators.json`
  - `_css_working/fixtures/selectors_specificity_id_wins.json`
  - `_css_working/fixtures/selectors_structural_pseudos.json`
  - `_css_working/fixtures/solid_red_block.json`
  - `_css_working/fixtures/transform_rotate_block_centered.json`
  - `_css_working/fixtures/transform_scale_block_centered.json`
  - `_css_working/fixtures/transform_skewx_block_left_origin.json`
  - `_css_working/fixtures/transform_matrix_block_affine.json`
  - `_css_working/fixtures/transform_individual_compose_with_transform.json`
  - `_css_working/fixtures/transform_var_reference_custom_list.json`
  - `_css_working/fixtures/transform_rotate_var_reference_custom_angle.json`
  - `_css_working/fixtures/transform_translate_var_reference_custom_pair.json`
  - `_css_working/fixtures/transform_scale_var_reference_custom_pair.json`
  - `_css_working/fixtures/transform_rotate_var_fallback_missing.json`
  - `_css_working/fixtures/transform_var_cycle_fallback_translate.json`
  - `_css_working/fixtures/transform_origin_left_scale_anchor.json`
  - `_css_working/fixtures/transform_translate_block_offsets.json`
  - `_css_working/fixtures/two_page_break_colors.json`
  - `_css_working/fixtures/units_calc_width_resolution.json`
  - `_css_working/fixtures/units_min_max_clamp_widths.json`
  - `_css_working/fixtures/position_relative_inset_shorthand_uniform.json`
  - `_css_working/fixtures/position_fixed_repeats_each_page.json`
  - `_css_working/fixtures/position_fixed_negative_zindex_under_content.json`
  - `_css_working/fixtures/position_fixed_zindex_front_ordering.json`
  - `_css_working/fixtures/position_absolute_root_zindex_overlay.json`
  - `_css_working/fixtures/position_absolute_root_page_one_only.json`
  - `_css_working/fixtures/position_absolute_root_negative_page_one_only.json`
  - `_css_working/fixtures/position_absolute_root_source_order_zindex.json`
  - `_css_working/fixtures/position_absolute_root_specificity_zindex_id_wins.json`
  - `_css_working/fixtures/position_absolute_left_right_width_precedence.json`
  - `_css_working/fixtures/position_absolute_top_bottom_height_precedence.json`
  - `_css_working/fixtures/position_absolute_empty_box_background.json`
  - `_css_working/fixtures/position_absolute_nearest_positioned_ancestor.json`
  - `_css_working/fixtures/position_absolute_percent_nearest_positioned_ancestor.json`
  - `_css_working/fixtures/position_absolute_initial_containing_block_from_static_parent.json`
  - `_css_working/fixtures/position_absolute_transform_establishes_containing_block.json`
  - `_css_working/fixtures/position_absolute_explicit_width_overflow_no_clamp.json`
  - `_css_working/fixtures/position_absolute_auto_inset_static_position_fallback.json`
  - `_css_working/fixtures/flex_place_items_container_alignment.json`
  - `_css_working/fixtures/flex_place_self_item_override.json`
  - coverage: `62/62` fixtures now include layout assertions.
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_RELATIVE_DRAW_PATH > position:relative draw-time offset path` implemented in:
  - `src/flowable.rs` (`RelativePositionedFlowable` wrapper executes `left/top/right/bottom` relative offsets at draw time while preserving normal flow slot geometry and pagination contracts).
  - `src/html.rs` (`position:relative` lowering wraps element flowables into `RelativePositionedFlowable`; `position:absolute` path remains out-of-flow).
  - fixtures:
    - `_css_working/fixtures/position_relative_offset_preserves_flow_slot.json`
    - `_css_working/fixtures/position_relative_right_bottom_offsets.json`
    - `_css_working/fixtures/inset_left_var_resolution.json` updated to assert first-class relative draw behavior.
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_RELATIVE_PRECEDENCE_PATH > position:relative precedence for opposing inset pairs` implemented with fixtures:
  - `_css_working/fixtures/position_relative_left_right_precedence.json` (`left` precedence when both `left` and `right` are present).
  - `_css_working/fixtures/position_relative_top_bottom_precedence.json` (`top` precedence when both `top` and `bottom` are present).
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_RELATIVE_PERCENT_CALC_PATH > position:relative percentage/calc inset resolution against containing width` implemented in:
  - `src/flowable.rs` (`RelativePositionedFlowable` now resolves horizontal inset percentages/calc expressions using container draw width basis instead of child box width).
  - fixtures:
    - `_css_working/fixtures/position_relative_percent_left_containing_width.json`
    - `_css_working/fixtures/position_relative_calc_left_containing_width.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_RELATIVE_PERCENT_HEIGHT_PATH > position:relative top percentage resolution against fixed containing height` implemented in:
  - `src/flowable.rs` (`Flowable::prefers_containing_block_draw_space` contract and `ContainerFlowable` draw-path hook pass containing-block height basis to relative wrappers when definite).
  - fixtures:
    - `_css_working/fixtures/position_relative_percent_top_containing_height.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_RELATIVE_CALC_RIGHT_BOTTOM_PATH > position:relative right/bottom calc+percentage resolution against containing dimensions` implemented with fixtures:
  - `_css_working/fixtures/position_relative_calc_right_containing_width.json`
  - `_css_working/fixtures/position_relative_calc_bottom_containing_height.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_FIXED_PAGED_REPEAT_PATH > position:fixed first-class paged-repeat rendering path` implemented in:
  - `src/style.rs` (`PositionMode::Fixed` and CSS position mapping now preserve fixed semantics in computed style instead of collapsing to absolute).
  - `src/html.rs` (position lowering now tags absolute wrappers as fixed-positioned when `position: fixed` is computed).
  - `src/flowable.rs` (`Flowable::is_fixed_positioned` contract and `AbsolutePositionedFlowable` fixed-position marker).
  - `src/doc_template.rs` (fixed overlays are separated from story flowables and composited on every page during page finalization, with deterministic z-index split for underlay/overlay behavior and ascending z-index layering within each fixed overlay lane).
  - style unit test:
    - `style::tests::position_fixed_maps_to_fixed_mode`
  - fixtures:
    - `_css_working/fixtures/position_fixed_repeats_each_page.json`
    - `_css_working/fixtures/position_fixed_negative_zindex_under_content.json`
    - `_css_working/fixtures/position_fixed_zindex_front_ordering.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_ROOT_ZINDEX_PATH > root-level position:absolute z-index overlay lane` implemented in:
  - `src/doc_template.rs` (non-fixed out-of-flow root story nodes are split into page-one back/front overlay lanes with deterministic ascending z-index ordering; `z-index < 0` paints before flow and `z-index >= 0` paints after flow).
  - fixture:
    - `_css_working/fixtures/position_absolute_root_zindex_overlay.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_ROOT_PAGE_SCOPE_PATH > root-level position:absolute page-one scope under pagination` implemented with fixture:
  - `_css_working/fixtures/position_absolute_root_page_one_only.json` (absolute root overlay paints on page one and does not repeat on subsequent pages).
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_ROOT_NEGATIVE_PAGE_SCOPE_PATH > root-level position:absolute z-index:-1 page-one underlay scope` implemented with fixture:
  - `_css_working/fixtures/position_absolute_root_negative_page_one_only.json` (negative z-index root underlay paints on page one and does not repeat on subsequent pages).
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_ROOT_CASCADE_SOURCE_ORDER_PATH > root-level position:absolute equal-specificity source-order z-index resolution path` implemented with fixture:
  - `_css_working/fixtures/position_absolute_root_source_order_zindex.json` (later equal-specificity selector deterministically wins `background` and `z-index`, and resulting overlay stacking is verified in paint assertions).
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_ROOT_SPECIFICITY_PATH > root-level position:absolute specificity-over-source-order cascade path` implemented with fixture:
  - `_css_working/fixtures/position_absolute_root_specificity_zindex_id_wins.json` (higher-specificity `#id` selector deterministically wins `background` and `z-index` over later lower-specificity class selector, with paint/layout assertions verifying overlay outcome).
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_OVERCONSTRAINT_PRECEDENCE_PATH > position:absolute explicit width/height precedence over opposing inset pairs` implemented in:
  - `src/flowable.rs` (`AbsolutePositionedFlowable` now preserves explicit `width`/`height` when both opposing insets are set, stretching only when size is `auto`; left/top anchoring remains deterministic in current LTR model).
  - `src/html.rs` (`wrap_absolute` now propagates computed `width`/`height` specs into absolute wrappers for draw-time over-constraint resolution).
  - fixtures:
    - `_css_working/fixtures/position_absolute_left_right_width_precedence.json`
    - `_css_working/fixtures/position_absolute_top_bottom_height_precedence.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_EMPTY_BOX_PATH > position:absolute empty-box background paint path` implemented in:
  - `src/html.rs` (empty inline-text fast paths now route through container boxing so width/height/background styles are preserved even when element text content is empty).
  - fixture:
    - `_css_working/fixtures/position_absolute_empty_box_background.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_NEAREST_POSITIONED_CB_PATH > position:absolute nearest-positioned-ancestor containing-block resolution path` implemented in:
  - `src/canvas.rs` (runtime containing-block context stack for nearest positioned ancestor resolution during draw).
  - `src/flowable.rs` (`ContainerFlowable` now marks positioned containers as containing-block providers and `AbsolutePositionedFlowable` resolves insets/sizing against the nearest containing-block context when present).
  - `src/html.rs` (positioned container lowering now tags style-derived containers as containing-block providers across block/flex/table/list lowering paths).
  - fixtures:
    - `_css_working/fixtures/position_absolute_nearest_positioned_ancestor.json`
    - `_css_working/fixtures/position_absolute_percent_nearest_positioned_ancestor.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_INITIAL_AND_TRANSFORM_CB_PATH > position:absolute initial containing-block fallback and transform-established containing-block path` implemented in:
  - `src/flowable.rs` (`AbsolutePositionedFlowable` falls back to page initial containing block when no containing-block context is present, instead of immediate static parent draw-space).
  - `src/html.rs` (`establishes_abs_containing_block` now includes non-`none` transforms in addition to non-static positioning).
  - fixtures:
    - `_css_working/fixtures/position_absolute_initial_containing_block_from_static_parent.json`
    - `_css_working/fixtures/position_absolute_transform_establishes_containing_block.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_EXPLICIT_SIZE_OVERFLOW_PATH > position:absolute explicit size overflow without containing-block clamp` implemented in:
  - `src/flowable.rs` (`AbsolutePositionedFlowable` no longer clamps explicit `width`/`height` to containing-block dimensions; explicit size is preserved with deterministic overflow behavior).
  - fixture:
    - `_css_working/fixtures/position_absolute_explicit_width_overflow_no_clamp.json`
- `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW > WS_POSITION_ABSOLUTE_STATIC_FALLBACK_PATH > position:absolute auto-inset static-position fallback path` implemented in:
  - `src/flowable.rs` (`ContainerFlowable` now computes deterministic static-position fallback coordinates for out-of-flow children from source-order in-flow cursor state; `AbsolutePositionedFlowable` uses passed fallback coordinates when opposing inset pair for an axis is fully `auto`).
  - fixture:
    - `_css_working/fixtures/position_absolute_auto_inset_static_position_fallback.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_FLEX_FLOW_SHORTHAND_PATH > flex-flow shorthand first-class compute+layout path` implemented in:
  - `src/style.rs` (`Property::FlexFlow` typed parsing path and `PropertyId::FlexFlow` unparsed path now map to computed `flex_direction`/`flex_wrap` state).
  - `src/style.rs` (`parse_flex_flow_str` deterministic shorthand normalization with default component fill and strict token handling).
  - `src/style.rs` (diagnostics alignment: `flex-flow` removed from parsed-no-effect lanes and marked engine-supported).
  - `src/style.rs` (computed debug emission now includes `flex_direction` for fixture-level compute assertions).
  - style unit tests:
    - `style::tests::flex_flow_shorthand_sets_direction_and_wrap`
    - `style::tests::flex_flow_shorthand_defaults_missing_component`
  - fixture:
    - `_css_working/fixtures/flex_flow_row_wrap_layout.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_ALIGN_CONTENT_PATH > align-content first-class wrapped-line distribution path` implemented in:
  - `src/style.rs` (`Property::AlignContent` typed parsing and `PropertyId::AlignContent` unparsed parsing now map to computed `align_content` state).
  - `src/style.rs` (diagnostics alignment: `align-content` removed from parsed-no-effect lanes and marked engine-supported).
  - `src/html.rs` (propagate computed `align_content` into `FlexFlowable` lowering).
  - `src/flowable.rs` (`FlexLayout::RowWrap` draw path applies `align-content` line packing for `flex-start`/`flex-end`/`center`/`space-between`/`space-around`/`space-evenly`).
  - style unit tests:
    - `style::tests::align_content_property_resolves`
    - `style::tests::align_content_distribution_keyword_resolves`
  - fixture:
    - `_css_working/fixtures/flex_align_content_flex_end_wrap.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_ALIGN_SELF_OVERRIDE_PATH > align-self first-class item-level cross-axis override` implemented in:
  - `src/style.rs` (`Property::AlignSelf` typed parsing and `PropertyId::AlignSelf` unparsed parsing now map to computed `align_self` state, with debug emission for fixture assertions).
  - `src/style.rs` (diagnostics alignment: `align-self` removed from parsed-no-effect lanes and marked engine-supported).
  - `src/html.rs` (child computed-style sampling in flex container lowering maps `align_self` into per-item override metadata).
  - `src/flowable.rs` (`FlexItem` now carries optional per-item align override; draw path resolves `align-self` override over container `align-items` for row/row-wrap/column execution).
  - style unit tests:
    - `style::tests::align_self_property_resolves`
    - `style::tests::align_self_auto_is_default`
  - fixture:
    - `_css_working/fixtures/flex_align_self_item_override.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_PLACE_ITEMS_SHORTHAND_PATH > place-items shorthand mapping into container align-items` implemented in:
  - `src/style.rs` (`Property::PlaceItems` typed parsing and `PropertyId::PlaceItems` unparsed parsing now route through `parse_place_items_align_str` into computed `align_items`).
  - `src/style.rs` (`parse_place_items_align_str` and `align_items_mode_from_keyword` provide deterministic one/two-value place-items parsing with stable align-items projection).
  - `src/style.rs` (diagnostics alignment: `place-items` removed from parsed-no-effect lanes and promoted to engine-supported compute behavior).
  - style unit tests:
    - `style::tests::place_items_shorthand_sets_align_items`
    - `style::tests::place_items_single_value_sets_align_items`
  - fixture:
    - `_css_working/fixtures/flex_place_items_container_alignment.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_PLACE_CONTENT_SHORTHAND_PATH > place-content shorthand first-class mapping into wrapped flex placement` implemented in:
  - `src/style.rs` (`Property::PlaceContent` typed parsing and `PropertyId::PlaceContent` unparsed parsing route into computed `align_content` + `justify_content`).
  - `src/style.rs` (`parse_place_content_str` deterministic shorthand parsing for one/two-value forms with keyword normalization).
  - `src/style.rs` (diagnostics alignment: `place-content` removed from parsed-no-effect lanes and marked engine-supported).
  - `src/style.rs` + `src/flowable.rs` + `src/html.rs` (`justify-content` support extended to `space-around` and `space-evenly` with deterministic draw-time spacing).
  - style unit tests:
    - `style::tests::justify_content_distribution_keywords_resolve`
    - `style::tests::place_content_shorthand_sets_align_and_justify`
    - `style::tests::place_content_single_value_copies_distribution_keyword`
  - fixture:
    - `_css_working/fixtures/flex_place_content_flex_end_center_wrap.json`
- `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX > WS_PLACE_SELF_SHORTHAND_PATH > place-self shorthand mapping into item-level align-self override` implemented in:
  - `src/style.rs` (`Property::PlaceSelf` typed parsing and `PropertyId::PlaceSelf` unparsed parsing now route through `parse_place_self_align_str` into computed `align_self`).
  - `src/style.rs` (`parse_place_self_align_str` and `align_self_mode_from_keyword` provide deterministic one/two-value place-self parsing with stable align-self projection).
  - `src/style.rs` (diagnostics alignment: `place-self` removed from parsed-no-effect lanes and promoted to engine-supported compute behavior).
  - style unit tests:
    - `style::tests::place_self_shorthand_sets_align_self`
    - `style::tests::place_self_single_value_sets_align_self`
  - fixture:
    - `_css_working/fixtures/flex_place_self_item_override.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSLATE_PATH > transform/translate first-class compute+draw path` implemented in:
  - `src/style.rs` (`transform` + `translate` parsing/computed-style resolution, CSS-wide keyword handling, debug-style emission).
  - `src/flowable.rs` (`CssTransformOp` and deterministic draw-time translation in `ContainerFlowable`).
  - `src/html.rs` (propagate computed transforms into container flowables).
  - style unit tests:
    - `style::tests::transform_translate_functions_resolve`
    - `style::tests::translate_longhand_resolves`
    - `style::tests::transform_unset_resets_to_none`
    - `style::tests::transform_inherit_uses_parent_value`
  - fixture:
    - `_css_working/fixtures/transform_translate_block_offsets.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_AFFINE_2D_PATH > transform scale/rotate first-class compute+draw path` implemented in:
  - `src/style.rs` (`transform` + `scale` + `rotate` parsing/computed-style resolution for typed and unparsed declarations, including longhands).
  - `src/flowable.rs` (`CssTransformOp::Scale` / `CssTransformOp::Rotate` draw-time application with deterministic transform-origin command ordering).
  - style unit tests:
    - `style::tests::transform_scale_and_rotate_functions_resolve`
    - `style::tests::rotate_longhand_resolves`
    - `style::tests::scale_longhand_resolves`
  - fixtures:
    - `_css_working/fixtures/transform_scale_block_centered.json`
    - `_css_working/fixtures/transform_rotate_block_centered.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_AFFINE_MATRIX_SKEW_PATH > transform skew/matrix first-class compute+draw path` implemented in:
  - `src/style.rs` (`transform` parsing/computed-style resolution for `skew`, `skewX`, `skewY`, `matrix`, and 2D-safe `matrix3d`, including unparsed fallback parsing).
  - `src/flowable.rs` (`CssTransformOp::Skew` / `CssTransformOp::Matrix` draw-time affine application).
  - `src/canvas.rs` (new `Command::ConcatMatrix` and `Canvas::concat_matrix` emission API).
  - command pipeline wiring:
    - `src/pdf.rs` emits generic affine `cm` for `ConcatMatrix`.
    - `src/raster.rs` applies `ConcatMatrix` to tiny-skia transform state.
    - `src/jit.rs` includes `ConcatMatrix` in bounds-transform reconstruction.
    - `src/spill.rs` serializes/deserializes `ConcatMatrix` deterministically.
  - style unit tests:
    - `style::tests::transform_skew_functions_resolve`
    - `style::tests::transform_matrix_function_resolves`
    - `style::tests::transform_matrix3d_2d_components_resolve`
    - `style::tests::transform_matrix3d_with_3d_terms_is_rejected`
  - fixtures:
    - `_css_working/fixtures/transform_skewx_block_left_origin.json`
    - `_css_working/fixtures/transform_matrix_block_affine.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_INDIVIDUAL_COMPOSITION_PATH > individual transform properties compose with transform` implemented in:
  - `src/style.rs` (property-scoped transform storage for `transform`/`translate`/`rotate`/`scale`, deterministic composition order, and scoped `inherit`/`initial` semantics).
  - style unit tests:
    - `style::tests::individual_transform_properties_compose_with_transform`
    - `style::tests::translate_inherit_is_scoped_to_translate_property`
    - `style::tests::transform_inherit_does_not_pull_translate_rotate_or_scale`
  - fixture:
    - `_css_working/fixtures/transform_individual_compose_with_transform.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSFORM_VAR_PATH > transform var()/fallback chains resolved through compute` implemented in:
  - `src/style.rs` (`pending_transform_var`/`pending_translate_var`/`pending_rotate_var`/`pending_scale_var` slots, custom-ref graph resolver for transform ops, and deterministic recomposition after pending resolution).
  - `src/style.rs` (custom-property declaration path now parses `transform`/`translate`/`rotate`/`scale` in unparsed lanes, with concrete-over-pending precedence in apply stage).
  - `src/style.rs` (custom token serialization now preserves `Angle` tokens in raw custom-property values, fixing `rotate: var(--r)` evaluation from first-class compute input).
  - style unit tests:
    - `style::tests::transform_var_reference_resolves_custom_transform_list`
    - `style::tests::transform_var_fallback_resolves_when_missing`
    - `style::tests::transform_var_cycle_uses_fallback`
    - `style::tests::translate_var_fallback_resolves_when_missing`
    - `style::tests::rotate_var_reference_resolves_custom_angle`
    - `style::tests::scale_var_reference_resolves_custom_pair`
    - `style::tests::concrete_transform_overrides_prior_pending_var`
  - fixture:
    - `_css_working/fixtures/transform_var_reference_custom_list.json`
    - `_css_working/fixtures/transform_rotate_var_reference_custom_angle.json`
    - `_css_working/fixtures/transform_translate_var_reference_custom_pair.json`
    - `_css_working/fixtures/transform_scale_var_reference_custom_pair.json`
    - `_css_working/fixtures/transform_rotate_var_fallback_missing.json`
    - `_css_working/fixtures/transform_var_cycle_fallback_translate.json`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSFORM_ORIGIN_PATH > transform-origin first-class compute+draw path` implemented in:
  - `src/style.rs` (`transform-origin` parsing for typed/unparsed declarations, CSS-wide keyword handling, computed-style debug emission).
  - `src/flowable.rs` (origin resolution against border box and deterministic origin shift around transform op sequence).
  - `src/html.rs` (propagate computed `transform_origin` into `ContainerFlowable` instances).
  - style unit tests:
    - `style::tests::transform_origin_keywords_resolve`
    - `style::tests::transform_origin_far_edge_offset_resolves`
    - `style::tests::transform_origin_unset_resets_to_center`
    - `style::tests::transform_origin_inherit_uses_parent_value`
  - fixture:
    - `_css_working/fixtures/transform_origin_left_scale_anchor.json`
- `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS > WS_VISUAL_VALIDATION > Manual PNG spot-check loop from emitted artifacts` executed via artifact outputs under `output/css_fixture_artifacts/*/output_page*.png`.

Current fixture lane state:

- iterative lane entrypoint: `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 ...` (defaults to no-build for rapid loops).
- explicit source-sync build lane: `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build ...` (maturin preferred, deterministic cargo fallback).
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels fast --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels full --jobs 3 --json` passes (62/62 fixtures).
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_fixed_repeats_each_page --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_fixed_zindex_front_ordering --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_root_zindex_overlay --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_root_page_one_only --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_root_negative_page_one_only --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_root_source_order_zindex --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_root_specificity_zindex_id_wins --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures flex_place_self_item_override --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures flex_place_items_container_alignment --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --fixtures position_absolute_left_right_width_precedence,position_absolute_top_bottom_height_precedence --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_empty_box_background,position_absolute_left_right_width_precedence,position_absolute_top_bottom_height_precedence --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_nearest_positioned_ancestor,position_absolute_percent_nearest_positioned_ancestor --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_initial_containing_block_from_static_parent,position_absolute_transform_establishes_containing_block,position_absolute_nearest_positioned_ancestor,position_absolute_percent_nearest_positioned_ancestor --update-stability --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_explicit_width_overflow_no_clamp --update-stability --emit-artifacts-dir output/css_fixture_artifacts --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --fixtures position_absolute_explicit_width_overflow_no_clamp --update-stability --emit-artifacts-dir output/css_fixture_artifacts --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures position_absolute_auto_inset_static_position_fallback --update-stability --emit-artifacts-dir output/css_fixture_artifacts --json` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --fixtures position_absolute_auto_inset_static_position_fallback --update-stability --emit-artifacts-dir output/css_fixture_artifacts --json` passes.
- `cargo check -q` passes.
- `powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels full --jobs 3 --json` passes (62/62 fixtures).
- `python tools/generate_css_parity_status.py --check --json` passes.

## Rust Core Examination (Current Theory Baseline)

Examined core files:

- `src/lib.rs`: render pipeline orchestration (`build_render_context` -> story -> layout -> plan -> finalize).
- `src/style.rs`: rule parsing/indexing, selector matching, cascade, computed style, media/@page extraction.
- `src/html.rs`: DOM -> `Flowable` lowering, pseudo-content insertion, display-mode mapping.
- `src/flowable.rs`: wrap/split/draw contracts for text, container, flex, table, relative/absolute positioning.
- `src/frame.rs`: frame placement state machine (`Placed`/`Split`/`Overflow`).
- `src/doc_template.rs`: multi-frame page loop and page-template selection.
- `src/types.rs`: deterministic fixed-point `Pt` based on `I32F32` and millipoint rounding.

Core implementation theory to preserve while expanding CSS:

1. `Deterministic numeric substrate`
- Layout decisions already run on `Pt` fixed-point values.
- Arithmetic should remain normalized at `Pt` boundaries to avoid float drift in split/wrap thresholds.

2. `Two-stage CSS interpretation`
- Stage A: parse declarations and selectors into rule deltas (`normal` and `!important` tracks).
- Stage B: compute per-element style via deterministic match ordering (specificity, then source order), then resolve pending vars.

3. `Flowable graph execution model`
- HTML/CSS lowers to a `Flowable` tree with strict `wrap -> split -> draw` semantics.
- Pagination correctness is distributed across individual flowables and `Frame`/`DocTemplate`.

4. `Pagination as a bounded state machine`
- `Frame::add` and `DocTemplate::build_with_metrics` provide deterministic page/frame transitions.
- CSS fragmentation features must compile into this state machine, not bypass it.

5. `Deterministic paint ordering`
- In-flow draw order, then out-of-flow z-index ordering, drives compositing stability.
- New paint/effects features must map to explicit order rules, never hash/iteration order.

6. `Known architecture gaps to close for parity`
- Selectors are currently parsed by a custom parser over serialized selector strings.
- `display:grid` is currently implemented through a grid-like flex path.
- Custom property resolution is partly specialized (length/color/font patterns) rather than fully generic token graph evaluation.
- Coverage for logical/writing-mode and advanced at-rules is partial.
- Effects/compositing parity is still partial (`filter`, `backdrop-filter`, `mix-blend-mode`) and multi-layer background image compositing currently resolves to a deterministic single-layer painter.

## Breadcrumbed Sprint Plan

### Phase P0 - Baseline and Instrumentation

1. `CSS_PARITY > P0_BASELINE > S00_PROGRAM_SETUP`
- Build MDN-module parity ledger (module -> properties -> status).
- Add `css_parity_status.json` artifact generation in CI.
- Add deterministic hash verification to all CSS-focused goldens.
- Implementation theory: tag each property with exact engine stage and file ownership:
  parser (`src/style.rs`), compute (`src/style.rs`), layout (`src/html.rs`/`src/flowable.rs`), pagination (`src/frame.rs`/`src/doc_template.rs`), paint (`src/flowable.rs`/`src/canvas.rs`).
- Exit criteria: baseline matrix exists, reproducibility gate is green.

2. `CSS_PARITY > P0_BASELINE > S01_TEST_HARNESS`
- Build CSS fixture runner for parser/computed/layout/paint stages.
- Introduce fixture metadata: `required_features`, `expected_warnings`, `stability_hash`.
- Add WPT/MDN fixture import pipeline with pinned snapshot metadata.
- Implementation theory: add stage-isolated assertions for:
  `StyleResolver::compute_style`, `Flowable::wrap/split/draw`, `DocTemplate::build_with_metrics`, and final PDF/image bytes.
- Exit criteria: end-to-end harness runs in CI with per-stage assertions.

### Phase P1 - Syntax, Selectors, Cascade

3. `CSS_PARITY > P1_CASCADE_CORE > S02_SYNTAX_AND_AT_RULES`
- Ensure robust parse/recovery for all major at-rules used by MDN CSS modules.
- Track unknown/unsupported at-rules as structured diagnostics, not silent drops.
- Expand custom property token preservation.
- Implementation theory: keep a dual-path declaration system:
  typed `Property::*` handling plus unparsed token fallback to avoid dropping valid-but-not-yet-modeled CSS.
- Exit criteria: parser parity score >= 95% for selected corpus.

4. `CSS_PARITY > P1_CASCADE_CORE > S03_SELECTORS_AND_SPECIFICITY`
- Complete selector support (attribute variants, combinators, pseudo-classes for static docs).
- Implement deterministic specificity + source-order tie-breaking.
- Add pseudo-element handling alignment (`::before`, `::after`, marker/content paths).
- Implementation theory: evolve from current custom selector parser into an AST-backed matcher while preserving current indexed lookup strategy (`by_id`, `by_class`, `by_tag`, universal) and deterministic candidate sorting.
- Exit criteria: selector conformance suite green for static selector set.

### Phase P2 - Values and Computed Style

5. `CSS_PARITY > P2_COMPUTED_VALUES > S04_UNITS_AND_CALC`
- Implement full value grammar coverage for lengths, percentages, angles, resolution units.
- Harden `calc()/min()/max()/clamp()` simplification and type checking.
- Add deterministic precision/rounding policy docs and tests.
- Implementation theory: normalize values into a canonical internal linear form (current `LengthSpec`/`CalcLength`) before layout; reject or downgrade non-linear expressions deterministically with diagnostics.
- Exit criteria: computed-value suite for units/functions reaches target parity.

6. `CSS_PARITY > P2_COMPUTED_VALUES > S05_INHERITANCE_AND_VARIABLES`
- Complete inheritance/initial/revert/unset/revert-layer handling.
- Finalize CSS custom properties resolution including fallback chains and cycles.
- Add cycle detection + deterministic bailout diagnostics.
- Implementation theory: replace property-specific var hooks with a generic token graph resolver, then project resolved tokens into typed properties; keep bounded recursion depth and stable failure behavior.
- Exit criteria: variable resolution corpus and inheritance matrix pass.

### Phase P3 - Layout Core

7. `CSS_PARITY > P3_LAYOUT_CORE > S06_BOX_MODEL_AND_FLOW`
- Close gaps in margin/padding/border sizing, box-sizing, overflow, positioning interactions.
- Improve block/inline formatting behavior and replaced element sizing paths.
- Normalize writing-mode sensitive dimensions in fixed-point space.
- Implementation theory: preserve cached layout by `(avail_width, avail_height)` keys and enforce one-way data flow:
  computed style -> resolved box metrics -> wrap/split/draw, with no hidden mutable geometry state.
- Exit criteria: box model and normal flow goldens stable.

8. `CSS_PARITY > P3_LAYOUT_CORE > S07_TEXT_AND_FONTS`
- Expand font matching, fallback chains, line-height, white-space, word-break, overflow-wrap.
- Add deterministic line-breaking and shaping configuration lock.
- Ensure stable glyph metrics mapping across environments.
- Implementation theory: keep deterministic fallback run segmentation and width caching; all line-break decisions must be derived from fixed-point width comparisons and explicit shaping config.
- Exit criteria: typography conformance suite and glyph stability tests pass.

### Phase P4 - Modern Layout Systems

9. `CSS_PARITY > P4_LAYOUT_SYSTEMS > S08_FLEXBOX`
- Complete flex item sizing algorithm edge cases (min/max constraints, flex-basis auto behavior).
- Add alignment/distribution correctness tests.
- Validate fragmentation behavior in paged context.
- Implementation theory: extend current `FlexFlowable` algorithm to a spec-faithful freeze/distribute loop implemented in deterministic arithmetic, then map split behavior onto existing `split_row_wrapped`/`split_column` semantics.
- Exit criteria: flexbox parity score reaches milestone target with no determinism drift.

10. `CSS_PARITY > P4_LAYOUT_SYSTEMS > S09_GRID`
- Implement track sizing algorithm parity (intrinsic sizing, minmax, auto-placement details).
- Cover alignment and spanning edge cases.
- Add page-fragmentation interaction policy for grid containers/items.
- Implementation theory: replace current grid-as-flex approximation (`grid_track_basis` + wrap) with a dedicated `GridFlowable` track solver and deterministic auto-placement order.
- Exit criteria: grid core suite green for targeted static/paged scenarios.

### Phase P5 - Visual and Paint Model

11. `CSS_PARITY > P5_PAINT > S10_BACKGROUND_BORDER_EFFECTS`
- Complete backgrounds, borders, border-radius, shadows, gradients, opacity layering semantics.
- Validate paint-order determinism and clipping semantics.
- Add blend/compositing policies as supported; warn deterministically on unsupported ops.
- Implementation theory: keep paint as an explicit ordered pipeline inside flowable draw passes:
  shadow -> background/gradient -> border -> children -> overlays, with deterministic z-order merges.
- Exit criteria: visual regression goldens stable across 3+ CI machines.

12. `CSS_PARITY > P5_PAINT > S11_LISTS_COUNTERS_GENERATED_CONTENT`
- Complete counters/list markers and generated content behavior.
- Align counter scope/reset/increment rules.
- Ensure pseudo-element content integrates with pagination and accessibility metadata.
- Implementation theory: model counters in style/context state and emit generated content through the same flowable lowering path currently used for `::before/::after` content.
- Exit criteria: counters and generated-content suite passes.

### Phase P6 - Paged Media and Print Fidelity

13. `CSS_PARITY > P6_PAGED_MEDIA > S12_FRAGMENTATION_AND_BREAKS`
- Finalize `break-before/after/inside`, widows/orphans, multiframe fragmentation.
- Stabilize table/list fragmentation decisions under fixed-point constraints.
- Add targeted regression bank for pathological pagination loops.
- Implementation theory: unify fragmentation policy across `Paragraph`, `ContainerFlowable`, `FlexFlowable`, and `TableFlowable` so `Frame::add` decisions are consistent and convergence is provable.
- Exit criteria: fragmentation suite passes with convergence guarantees.

14. `CSS_PARITY > P6_PAGED_MEDIA > S13_PRINT_RULES_AND_HARDENING`
- Complete `@media print`, `@page` behavior used by document workflows.
- Harden deterministic compose order and diagnostics for unsupported CSS.
- Freeze v1 CSS parity matrix and publish known limitations.
- Implementation theory: keep print/page setup extraction (`extract_css_page_setup`) and template resolution (`resolve_page_templates_for_css`) as the single source of truth for page geometry overrides.
- Exit criteria: parity release candidate with documented compatibility contract.

## MDN Module Coverage Map (Execution Backlog)

Prioritize in this sequence (each tracked by parser/compute/layout/paint parity columns):

1. Syntax and at-rules
2. Selectors
3. Cascade and inheritance
4. Values and units
5. Box model
6. Display and formatting contexts
7. Positioning
8. Sizing
9. Text and fonts
10. Backgrounds and borders
11. Lists and counters
12. Overflow
13. Flexbox
14. Grid
15. Tables
16. Transforms and coordinate spaces
17. Filters/effects/compositing (static subset + deterministic fallback)
18. Multi-column
19. Fragmentation
20. Paged media (`@page`, print constraints)
21. Writing modes and logical properties
22. Custom properties and properties/values API-adjacent parsing

## Risk Register

- `R1`: "All CSS" includes runtime-interactive behavior that is not meaningful for static PDF.
- `R2`: Non-deterministic text shaping differences across platform/font stacks.
- `R3`: Grid/flex edge cases can create fix-point non-convergence loops.
- `R4`: External corpus drift (WPT/MDN examples changing over time).

Mitigations:

- Encode explicit static-document compatibility contract.
- Pin corpus snapshots; version fixtures with hash locks.
- Add convergence step caps + deterministic bailout semantics.
- Run cross-platform determinism CI on every milestone.

## Metrics And Gates

- `Parity Coverage %`: implemented properties / targeted properties by module.
- `Render Stability %`: identical output hashes across repeated/cross-agent runs.
- `Conformance Pass %`: parser + computed + layout + paint fixtures passing.
- `Fallback Clarity %`: unsupported feature diagnostics with explicit reason/action.

Release gates:

1. No open P0/P1 determinism issues.
2. >= 90% conformance pass on targeted static/paged CSS corpus.
3. >= 99.9% reproducibility across CI reruns for golden fixtures.
4. Published compatibility matrix and limitation notes.
