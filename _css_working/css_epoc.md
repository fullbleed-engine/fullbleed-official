# CSS Parity EPOC (Sprint 1)

Source taxonomy: https://developer.mozilla.org/en-US/docs/Web/CSS

## Objective

Deliver a concrete end-to-end parity increment where CSS is evaluated as a first-class rendering participant (parser -> compute -> draw), not only parsed.

Sprint-1 scope targets `CSS transforms` phase-1 support for deterministic static rendering, including `transform-origin`, `skew`, `matrix`, and individual transform-property composition (`translate`/`rotate`/`scale` + `transform`).

## Breadcrumb

- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSLATE_PATH > Implement transform: translate(...) through parser/compute/draw`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_AFFINE_2D_PATH > Extend transform to scale/rotate with draw-time origin-aware application`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSFORM_ORIGIN_PATH > Implement transform-origin through parser/compute/draw`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_AFFINE_MATRIX_SKEW_PATH > Implement skew/matrix affine path through parser/compute/draw`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_INDIVIDUAL_COMPOSITION_PATH > Compose translate/rotate/scale with transform in property-scoped cascade order`
- `CSS_PARITY > P5_PAINT > S10_TRANSFORMS_PHASE1 > WS_TRANSFORM_VAR_PATH > Resolve transform var()/fallback chains through compute into draw-time ops`

## Implementation Theory (Rust Core)

- `src/style.rs`: parse and compute a deterministic transform model.
  - Add transform representation in computed style.
  - Parse `transform` declarations (phase-1 subset: translate/scale/rotate/skew/matrix families).
  - Parse individual longhands (`translate`, `scale`, `rotate`) to the same computed op model.
  - Compose individual transform properties with `transform` in deterministic property order (`translate` -> `rotate` -> `scale` -> `transform`) with property-scoped `inherit`/`initial` handling.
  - Resolve `transform` and individual transform-property `var(...)` expressions (including fallback/cycle-safe chains) through compute-time pending var resolution.
  - Parse and compute `transform-origin` (typed and unparsed paths, including CSS-wide keywords).
  - Accept 2D-compatible `matrix3d(...)` and reject non-2D matrix3d deterministically.
  - Honor CSS-wide keywords (`initial`, `unset`, `revert`, `revert-layer`, `inherit`) deterministically.
- `src/html.rs`: carry computed transform into the flowable graph.
  - Apply transform metadata when lowering element flowables.
  - Ensure transformed elements remain grouped so transform applies to the element box as a unit.
- `src/flowable.rs`: execute transform in draw stage.
  - Add deterministic transform application in draw (`save_state -> origin shift -> op sequence -> origin restore -> draw -> restore_state`).
  - Preserve existing layout invariants (transform does not alter wrap/split geometry in phase-1).
- Determinism constraints:
  - No floating-point branching in layout decisions.
  - Stable transform command ordering in draw path.
  - Unsupported transform functions degrade predictably (no nondeterministic partial behavior).

## Validation Paradigm (Formalized for Sprint 1)

- `V0 Unit`
  - Style parser/computed tests for transform parse + CSS-wide keyword behavior.
- `V1 Stage`
  - Fixture `compute_assertions` proving computed transform + transform-origin state.
  - Fixture `layout_assertions` proving translate/scale/rotate/skew/matrix/origin geometry from PNG scanlines.
- `V2 Visual`
  - Manual PNG spot-check from emitted artifact pages.
- `V3 Determinism`
  - Fixture stability hash match and rerun parity.

Required sprint command lane:

```powershell
cargo check -q
cargo test -q style::tests::transform_
cargo test -q style::tests::transform_matrix_
cargo test -q style::tests::transform_skew_functions_resolve
cargo test -q style::tests::transform_origin_
cargo test -q style::tests::rotate_longhand_resolves
cargo test -q style::tests::scale_longhand_resolves
cargo test -q style::tests::individual_transform_properties_compose_with_transform
cargo test -q style::tests::translate_inherit_is_scoped_to_translate_property
cargo test -q style::tests::transform_inherit_does_not_pull_translate_rotate_or_scale
cargo test -q style::tests::transform_var_reference_resolves_custom_transform_list
cargo test -q style::tests::transform_var_fallback_resolves_when_missing
cargo test -q style::tests::transform_var_cycle_uses_fallback
cargo test -q style::tests::rotate_var_reference_resolves_custom_angle
cargo test -q style::tests::translate_var_fallback_resolves_when_missing
cargo test -q style::tests::scale_var_reference_resolves_custom_pair
powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 -Build --fixtures transform_translate_block_offsets,transform_scale_block_centered,transform_rotate_block_centered,transform_origin_left_scale_anchor,transform_matrix_block_affine,transform_skewx_block_left_origin,transform_individual_compose_with_transform,transform_var_reference_custom_list,transform_rotate_var_reference_custom_angle,transform_translate_var_reference_custom_pair,transform_scale_var_reference_custom_pair,transform_rotate_var_fallback_missing,transform_var_cycle_fallback_translate --emit-artifacts-dir output/css_fixture_artifacts --json
powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels fast --json
powershell -ExecutionPolicy Bypass -File tools/run_css_fixture_pipeline.ps1 --labels full --jobs 3 --json
python tools/generate_css_parity_status.py --json
python tools/generate_css_parity_status.py --check --json
```

## Deliverables

- Transform phase-1 implementation in core Rust pipeline, including `transform-origin`, `skew`, `matrix`, individual-property composition, and transform var-chain resolution.
- New fixtures covering parser/compute/layout/paint for transform translate/scale/rotate/skew/matrix/origin/composition/var behavior.
- Parity ledger/status update for `transforms_coordinates` from `not_started` to `in_progress (partial stage coverage)`.
- Sprint documentation and validation artifacts under `_css_working/`.

## Exit Criteria

- Transform translate/scale/rotate/skew/matrix/origin/composition/var paths are active in rendered output (visual and pixel assertions pass).
- No regression in fast fixture lane.
- Parity status artifact stays in sync with ledger.
