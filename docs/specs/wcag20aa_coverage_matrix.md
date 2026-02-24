# WCAG 2.0 AA Coverage Matrix (Current)

This document summarizes current FullBleed coverage against the `WCAG 2.0 AA` target registry in `docs/specs/wcag20aa_registry.v1.yaml`.

It is a planning artifact for implementation sequencing, not a conformance claim.

## Basis

- Source of truth: `docs/specs/wcag20aa_registry.v1.yaml`
- Target scope: `38` WCAG 2.0 A/AA success criteria + `5` conformance requirements (`43` total)
- Entry implementation state (derived):
  - `implemented`: at least one `fullbleed_rule_mapping.status == "implemented"`
  - `planned_only`: has mappings, but none implemented
  - `unmapped`: no mappings

## Summary

- `43` total target entries
- `43` entries have any FullBleed mapping (`100.0%`)
- `43` entries have at least one implemented mapping (`100.0%`)
- `0` entries are currently unmapped (`0.0%`)

Breakdown:

- Success criteria: `38 / 38` implemented (`100.0%`)
- Conformance requirements: `5 / 5` implemented (`100.0%`)

## Matrix: By Kind x Implementation State

| Kind | Implemented | Planned Only | Unmapped | Total |
|---|---:|---:|---:|---:|
| Success Criterion | 38 | 0 | 0 | 38 |
| Conformance Requirement | 5 | 0 | 0 | 5 |
| Total | 43 | 0 | 0 | 43 |

## Matrix: Success Criteria by Principle x Implementation State

| Principle | Implemented | Planned Only | Unmapped | Total |
|---|---:|---:|---:|---:|
| Perceivable | 14 | 0 | 0 | 14 |
| Operable | 12 | 0 | 0 | 12 |
| Understandable | 10 | 0 | 0 | 10 |
| Robust | 2 | 0 | 0 | 2 |
| Total | 38 | 0 | 0 | 38 |

## Matrix: Verification Mode x Implementation State (All Entries)

| Verification Mode | Implemented | Planned Only | Unmapped | Total |
|---|---:|---:|---:|---:|
| Machine | 3 | 0 | 0 | 3 |
| Hybrid | 40 | 0 | 0 | 40 |
| Manual | 0 | 0 | 0 | 0 |
| Total | 43 | 0 | 0 | 43 |

Interpretation:

- We already cover the small machine-checkable core.
- The highest leverage near-term gains are in `hybrid` criteria.
- `manual` criteria remain backlog and should be represented as explicit evidence obligations, not silently omitted.

## Currently Implemented-Mapped WCAG Entries

- `wcag20.sc.1.3.1` (Info and Relationships) - partial
- `wcag20.sc.1.3.2` (Meaningful Sequence) - partial (sequence review evidence seed)
- `wcag20.sc.1.3.3` (Sensory Characteristics) - partial (heuristic seed)
- `wcag20.sc.1.1.1` (Non-text Content) - partial
- `wcag20.sc.1.2.1` (Audio-only and Video-only (Prerecorded)) - partial (media alternative review evidence seed)
- `wcag20.sc.1.2.2` (Captions (Prerecorded)) - partial (prerecorded captions review evidence seed)
- `wcag20.sc.1.2.3` (Audio Description or Media Alternative (Prerecorded)) - partial (prerecorded AD/media-alt review evidence seed)
- `wcag20.sc.1.2.4` (Captions (Live)) - partial (live captions review evidence seed)
- `wcag20.sc.1.2.5` (Audio Description (Prerecorded)) - partial (prerecorded audio-description review evidence seed)
- `wcag20.sc.1.4.1` (Use of Color) - partial (color-only meaning evidence seed)
- `wcag20.sc.1.4.2` (Audio Control) - partial (audio playback control evidence seed)
- `wcag20.sc.1.4.3` (Contrast (Minimum)) - partial (render-based seed)
- `wcag20.sc.1.4.4` (Resize text) - partial (resize-text review evidence seed)
- `wcag20.sc.1.4.5` (Images of Text) - partial (images-of-text review evidence seed)
- `wcag20.sc.2.1.1` (Keyboard) - partial (interactive evidence seed + structural keyboard-risk signals)
- `wcag20.sc.2.1.2` (No Keyboard Trap) - partial (interactive evidence seed)
- `wcag20.sc.2.2.1` (Timing Adjustable) - partial (timed-interaction evidence seed)
- `wcag20.sc.2.2.2` (Pause, Stop, Hide) - partial (moving/blinking/updating content evidence seed)
- `wcag20.sc.2.3.1` (Three Flashes or Below Threshold) - partial (flashing-content evidence seed)
- `wcag20.sc.2.4.1` (Bypass Blocks) - partial
- `wcag20.sc.2.4.2` (Page Titled)
- `wcag20.sc.2.4.3` (Focus Order) - partial (tabindex risk-signal seed)
- `wcag20.sc.2.4.4` (Link Purpose (In Context)) - partial
- `wcag20.sc.2.4.5` (Multiple Ways) - partial (page-set evidence seed)
- `wcag20.sc.2.4.6` (Headings and Labels) - partial
- `wcag20.sc.2.4.7` (Focus Visible) - partial (CSS focus/outline suppression seed)
- `wcag20.sc.3.1.1` (Language of Page)
- `wcag20.sc.3.1.2` (Language of Parts) - partial (inline lang declaration validity seed)
- `wcag20.sc.3.2.1` (On Focus) - partial (interactive focus behavior evidence seed)
- `wcag20.sc.3.2.2` (On Input) - partial (form-control behavior evidence seed)
- `wcag20.sc.3.2.3` (Consistent Navigation) - partial (page-set evidence seed)
- `wcag20.sc.3.2.4` (Consistent Identification) - partial (review-evidence seed)
- `wcag20.sc.3.3.1` (Error Identification) - partial
- `wcag20.sc.3.3.2` (Labels or Instructions) - partial
- `wcag20.sc.3.3.3` (Error Suggestion) - partial (form-flow evidence seed)
- `wcag20.sc.3.3.4` (Error Prevention (Legal, Financial, Data)) - partial (transactional/legal data form-flow evidence seed)
- `wcag20.sc.4.1.1` (Parsing) - partial
- `wcag20.sc.4.1.2` (Name, Role, Value) - partial
- `wcag20.conf.level` - partial (claim readiness scaffold)
- `wcag20.conf.full_pages` - partial
- `wcag20.conf.complete_processes` - partial (process-scope claim scaffold)
- `wcag20.conf.accessibility_supported_technologies` - partial (technology-support claim scaffold)
- `wcag20.conf.non_interference` - partial (hybrid seed scan + manual evidence path)

## Near-Term Automation Candidates (Not Yet Implemented)

All `machine|hybrid` entries in the WCAG registry now have at least partial implemented coverage.

Next gains require:
- deepening existing hybrid audits (higher confidence, lower false positive/negative rates), especially `2.1.1 Keyboard`, `1.4.3 Contrast`, and `1.3.2 Meaningful Sequence`.

## Conformance Requirement Gap Status

| Conformance Requirement | State | Notes |
|---|---|---|
| `wcag20.conf.level` | implemented (partial) | Claim-readiness scaffold reports machine blockers, coverage gaps, and manual-evidence requirements; it does not assert conformance |
| `wcag20.conf.full_pages` | implemented (partial) | PMR page parity and full-page reporting support the base signal; needs stronger claim linkage |
| `wcag20.conf.complete_processes` | implemented (partial) | Hybrid process-scope scaffold marks transactional/profile claims as manual-evidence-required and non-process claims not applicable |
| `wcag20.conf.accessibility_supported_technologies` | implemented (partial) | Hybrid seed records technology-risk signals and manual evidence requirement; does not prove AT/environment support |
| `wcag20.conf.non_interference` | implemented (partial) | Hybrid seed scan flags active-content risk signals and routes to manual evidence review; does not prove runtime non-interference |

## Section 508 Status (Current)

- A scoped Section 508 HTML/E205 profile registry now exists: `docs/specs/section508_html_registry.v1.yaml`
- Engine and prototype verifier reports now emit `coverage.section508` for that scoped profile.
- Current Section 508 coverage is **profile-scoped**, not full-suite:
  - includes `E205` scoping/claim entries
  - incorporates WCAG 2.0 AA coverage by reference (`E205.4`)
- Current scoped profile coverage (mapping-level):
  - `49 / 49` implemented-mapped entries (`100.0%`)
  - `6 / 6` Section 508-specific profile entries implemented (partial/manual-evidence seeds)
  - `43 / 43` inherited WCAG entries implemented
- Conclusion: FullBleed still does **not** test the full Section 508 suite across all ICT domains, but Section 508 coverage is now measurable for the HTML/E205 profile.

## Sprint 5 Prioritization Input (Rule Work)

Use this matrix to order Sprint 5 work after namespace/dedup foundations:

1. Deepen `wcag20.sc.1.4.3` contrast analysis (sampling strategy, confidence tiers, manual evidence linkage)

Defer unless sprint capacity remains:

- additional WCAG conformance/process requirements beyond current machine/hybrid seeds

Recently completed:

- `wcag20.sc.2.4.6` (Headings and Labels) partial machine/hybrid coverage via `fb.a11y.headings_labels.present_nonempty`
- `wcag20.sc.1.1.1` (Non-text Content) partial machine coverage via `fb.a11y.images.alt_or_decorative`
- `wcag20.sc.3.3.2` (Labels or Instructions) partial hybrid coverage via `fb.a11y.forms.labels_or_instructions_present`
- `wcag20.sc.3.3.1` (Error Identification) partial hybrid coverage via `fb.a11y.forms.error_identification_present`
- `wcag20.conf.level` (Conformance Level) partial claim-readiness scaffold via `fb.a11y.claim.wcag20aa_level_readiness`
- `wcag20.conf.non_interference` (Non-Interference) partial hybrid seed scan via `fb.a11y.claim.non_interference_seed`
- `wcag20.sc.1.4.3` (Contrast Minimum) partial render-based hybrid seed via `fb.a11y.contrast.minimum_render_seed`
- `wcag20.sc.2.4.4` (Link Purpose (In Context)) partial hybrid seed via `fb.a11y.links.purpose_in_context`
- `wcag20.sc.2.4.3` (Focus Order) partial hybrid seed via `fb.a11y.focus.order_seed`
- `wcag20.sc.2.4.5` (Multiple Ways) partial hybrid seed via `fb.a11y.navigation.multiple_ways_seed`
- `wcag20.sc.2.1.1` (Keyboard) partial hybrid seed via `fb.a11y.keyboard.operable_seed`
- `wcag20.sc.1.3.3` (Sensory Characteristics) partial hybrid seed via `fb.a11y.instructions.sensory_characteristics_seed`
- `wcag20.sc.3.1.2` (Language of Parts) partial hybrid seed via `fb.a11y.language.parts_declared_valid_seed`
- `wcag20.sc.3.2.3` (Consistent Navigation) partial hybrid seed via `fb.a11y.navigation.consistent_navigation_seed`
- `wcag20.sc.3.2.4` (Consistent Identification) partial hybrid seed via `fb.a11y.identification.consistent_identification_seed`
- `wcag20.sc.2.4.7` (Focus Visible) partial hybrid seed via `fb.a11y.focus.visible_seed`
- `wcag20.conf.complete_processes` partial hybrid process-scope scaffold via `fb.a11y.claim.complete_processes_scope_seed`
- `wcag20.conf.accessibility_supported_technologies` partial hybrid technology-support scaffold via `fb.a11y.claim.accessibility_supported_technologies_seed`
