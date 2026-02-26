# Paged Overflow Integrity Sprint

## Working Title

No Silent Overprint / Overflow (Paged Media Integrity)

## Decision (Policy)

Default policy: **no silent overprint/overflow on paged media**.

Agreed. A document should not be considered acceptable if text or other authored content:

- overprints sibling content
- overflows its container into adjacent regions
- is clipped/truncated without an explicit, intentional policy
- extends beyond page bounds

This is a layout-integrity invariant, not just a visual polish issue.

## Clarification: Wrap vs Shrink vs Reflow

The requirement is **no overflow**, not “always wrap” and not “always shrink”.

Preferred remediation order (default):

1. Wrap (preserve readable font size)
2. Reflow layout (grow container, move section, split across page)
3. Split content structurally (table/page split, paragraph continuation)
4. Bounded shrink (opt-in, within min font-size / readability constraints)
5. Fail-fast (if none of the above are valid for the content type)

### Why not always shrink?

- Can harm readability/accessibility (`WCAG 1.4.4` resize text expectations)
- Can break source-use parity in forms (line placement, line labels, legal readability)
- Can hide layout problems that should be fixed structurally

### Why not always wrap?

Some content should not be arbitrarily wrapped:

- fixed tokens / identifiers / certificate numbers / hashes / codes
- signatures, stamps, seals, barcodes/QR (visual marks should be scaled/contained)
- some preformatted blocks where whitespace is meaningful

For these, the rule is still **no overflow**; the remediation is not necessarily wrap.

## Problem Statement

Current observability catches page-bound overflow and some known-loss events, but it does not reliably catch:

- intra-page overlap (content from one region overprinting another)
- text exceeding container bounds while the container remains inside the page
- clipping/truncation inside a valid page placement

This allows visible layout defects to pass component mount validation and PMR, unless manually spotted.

Thunderbird CAV exposed this gap:

- legal description authored as `<pre>` in a narrow grid column rendered as non-wrapping `Code`
- text overprinted adjacent “Instrument filing stamp” column
- `overflow_count = 0` because page-bound placement overflow did not occur

## Sprint Goal

Make paged layout integrity fail-fast for overlap/overflow classes that currently escape detection, using engine-native non-visual observability and PMR integration.

## Non-Goals (This Sprint)

- Full auto-remediation engine (global layout solver)
- Perfect typographic optimization
- General-purpose page-break authoring API redesign
- Pixel-perfect collision detection for every shape/image primitive

## Implementation Theory

The fix should be **engine-observable**, not just CSS linting.

Why:

- CSS can be valid and still produce collisions (Thunderbird `<pre>` case)
- We need post-layout facts, not only pre-layout heuristics
- PMR should gate on rendered geometry risk, not inferred author intent

Therefore:

1. Add render-time geometry traces for text/container placements
2. Derive deterministic collision/overflow metrics from those traces
3. Surface them in component mount validation + PMR
4. Add authoring-risk lints as a secondary early-warning layer

## Detection Taxonomy (v1)

We need distinct metrics and error codes for these classes:

1. `page_boundary_overflow`
- placement extends beyond page bounds
- already partially covered (`overflow_count`)

2. `inter_box_overlap`
- content draw bbox overlaps a sibling container region or sibling content block unexpectedly
- the Thunderbird-class issue

3. `text_container_overflow`
- text draw bbox exceeds the intended container/cell bbox
- may or may not overlap visible neighboring content

4. `clipped_text_risk`
- content likely clipped by container/clip region or fixed-height region

5. `nowrap_risk_in_narrow_region` (authoring lint)
- semantic risk signal only (not a rendered proof)

## Sprint Scope (Execution)

### Workstream 1: Spec / Policy Freeze

Define explicit no-overflow contract and remediation policy.

Deliverables:

- `docs/specs/paged_layout_integrity.v1.md` (new)
- error code list / metric names
- PMR audit IDs and gate levels for v1

Acceptance:

- “No silent overflow” is documented as invariant
- remediation order is explicit (wrap/reflow/split/shrink/fail)

### Workstream 2: Engine Render-Time Geometry Trace (v1)

Extend render-time traces to emit enough geometry for overlap detection.

Minimum required trace facts:

- text draw bbox (x/y/w/h) per draw command
- container/box/grid-cell planned bbox (or nearest authored region bbox if available)
- page index
- z/order or command index
- top tag role / tag path (already present for some text traces)
- origin hint (text vs border/line/image)

Notes:

- We do not need perfect geometry for every primitive in v1.
- Text + container boxes + command order is enough to close the current gap.

Acceptance:

- Trace JSONs include non-null text draw dimensions for engine-rendered text
- At least one container-region geometry stream is available for collision checks

### Workstream 3: Geometry Analyzers (v1)

Implement engine-side (or wrapper-side initially, but trace-backed) analyzers:

- `inter_box_overlap_count`
- `text_container_overflow_count`
- `clipped_text_risk_count` (seed)

Evidence payloads should include samples:

- page index
- offending text excerpt (truncated)
- text bbox
- target/sibling bbox
- overlap ratio / overflow axis
- command/tag metadata

Acceptance:

- Thunderbird pre-patch overlap is detected by at least one metric
- Thunderbird patched version reports `0` for overlap metrics

### Workstream 4: Component Mount Validation Integration

Extend `validate_component_mount(...)` output to include new metrics and failure modes.

Proposed additions:

- `inter_box_overlap_count`
- `text_container_overflow_count`
- `clipped_text_risk_count`
- `layout_collision_samples`

Failure toggles (new):

- `fail_on_inter_box_overlap` (default `True` for accessibility harnesses)
- `fail_on_text_container_overflow` (default `True`)
- `fail_on_clipped_text_risk` (default `False` in v1; warning by default)

Acceptance:

- Thunderbird overlap reproducer fails component mount validation pre-patch
- current Thunderbird CAV passes after patch

### Workstream 5: PMR Integration (Paged Media Rank)

Add PMR audits (seed + required as appropriate):

- `pmr.layout.inter_box_overlap_none` (required, fail on count > 0)
- `pmr.layout.text_container_overflow_none` (required, fail on count > 0)
- `pmr.layout.clipped_text_risk_none_seed` (warn/manual in v1)

Category:

- `paged-layout-integrity`

Acceptance:

- PMR fails on Thunderbird overlap even when page-bound overflow is zero
- PMR evidence points to exact page/region/sample

### Workstream 6: Authoring Risk Lints (Secondary)

Add lightweight risk diagnostics in verifier/PMR preflight:

- `<pre>` / `white-space:nowrap` in grid/table narrow columns
- fixed-height content boxes with text descendants
- `overflow:hidden` on text containers

These do not replace geometry checks.

Acceptance:

- Risk warnings are surfaced with selectors/class names
- No false “pass” claims from lint-only signals

### Workstream 7: Regression Corpus + Gates

Use the existing 3-track corpus plus Thunderbird:

Tracks:

- `cav_golden` (expected pass)
- `known_failure_legacy` (expected PMR failure)
- `wild_best_attempt_html` (mixed)
- `thunderbird_cav` (expected pass after patch; historical overlap reproducer retained)

Gates:

- Thunderbird overlap fixture (pre-patch snapshot or synthetic clone) must fail overlap audit
- Thunderbird current CAV must pass overlap audits
- Golden CAV must not regress on overlap metrics

## Sprint Deliverables

1. New spec doc for paged layout integrity invariant
2. Engine trace extensions (geometry)
3. New mount-validation metrics and failure toggles
4. PMR overlap/overflow audits with evidence
5. Thunderbird regression proving the original gap is caught
6. Docs updates (how to interpret layout-integrity failures)

## Failure Gates (Sprint)

Required to merge:

- Thunderbird reproducer triggers overlap/overflow metric (non-zero)
- Thunderbird patched CAV has:
  - `inter_box_overlap_count = 0`
  - `text_container_overflow_count = 0`
- No regressions in existing golden marriage CAV PMR/verifier/PDF-UA seed status
- New metrics appear in component mount validation output and bundle run report

## Observability Requirements

Artifacts (versioned, CI-stable enough for presence/shape checks):

- `*_layout_collision_trace.json` (if separate trace is used)
- or enriched `*_reading_order_trace_render.json` / structure trace with bbox data

Run-report additions:

- `inter_box_overlap_count`
- `text_container_overflow_count`
- `clipped_text_risk_count`
- `layout_collision_sample_count`

PMR evidence:

- sample excerpts + bboxes, not just counts

## Open Questions (SME defaults)

1. Should overlap detection consider line/border intersections?
- Default: no (v1), focus on text-vs-content/container collisions

2. Should decorative overlays/watermarks be exempt?
- Default: yes, if explicitly marked decorative/background and not covering readable text

3. Should bounded shrink be automatic in the engine?
- Default: no automatic global shrink in v1; prefer fail-fast + authoring/layout correction

## Backlog (Post-Sprint)

- Auto-remediation policies per content type (wrap/split/shrink)
- True page-break authoring primitives with hard page boundary semantics
- Rich image/shape collision detection
- Table cell split heuristics for long legal/tabular content

## Recommendation

Treat this sprint as blocking for paged-media confidence.

The Thunderbird issue demonstrated that “page-bounds overflow = 0” is not enough. We need **collision-aware layout integrity** metrics before we trust PMR/layout pass signals on complex forms/CAVs.
