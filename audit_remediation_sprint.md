# Audit Remediation Sprint (Third-Party Accessibility Review)

Date: March 3, 2026  
Source audit: `accessibility_audit.md` (Jhanger Urdaneta, March 2026)

## 1. Problem Framing

The third-party audit confirms our HTML output is functionally navigable and reading order is correct.  
The remaining issues are optimization-level but materially impact screen-reader efficiency and cognitive load:

1. Excessive `alt` verbosity in figures.
2. Redundant `alt` + `figcaption` duplication.
3. Fragmented description lists (`dl/dt/dd`) where a unified list is preferable.
4. Unnecessary ARIA on elements with sufficient native HTML semantics.

This sprint targets "same use + low-friction assistive UX", not visual parity work.

## 2. Remediation Objectives

1. Reduce duplicate/redundant announcements for NVDA/AT users.
2. Make semantics canonical and enforceable at authoring time.
3. Add machine-verifiable gates so regressions fail fast in CI.
4. Keep native HTML-first behavior (`First Rule of ARIA`) as a hard default.

## 3. Implementation Theory

### 3.1 Announce-Once Principle
For figure/media content, each semantic fact should be announced once unless repetition is intentional.  
Implication: `alt` should be concise; `figcaption` should add non-duplicate context.

### 3.2 Native-First Semantics
Prefer native tags (`figure`, `figcaption`, `dl`, `dt`, `dd`, `table`, `th`, `label`) over ARIA overlays.  
Implication: ARIA is additive only when native semantics are insufficient.

### 3.3 Structured Group Integrity
Semantically related fields should be grouped in one coherent container when possible.  
Implication: avoid adjacent fragmented `dl` clusters that act like one logical list.

### 3.4 Measurable UX Debt
Optimization issues must emit metrics and rules, not only human notes.

### 3.5 Verifier Placement Model (Code-Level)
The shortest path is to add rules in the prototype verifier layer, then flow them through native engine reports:

1. Parse-layer facts:
   - `python/fullbleed/audit_prototype.py`
   - `class HtmlFacts`
   - `class _P(HTMLParser)` + `parse_html_facts(...)`
2. Rule emission:
   - `prototype_verify_accessibility(...)`
   - `prototype_verify_paged_media_rank(...)`
3. Native engine report bridge:
   - `src/lib.rs` (core verify facts/signals)
   - `src/python.rs` (`verify_accessibility_artifacts`, `verify_paged_media_rank_artifacts`)
   - `python/fullbleed/accessibility/engine.py` (bundle/run-report wiring)

### 3.6 Immutable Contract Theory
All new rule IDs must be represented in the audit registry contract to keep builds defensible and fingerprint-stable.

Required registry touchpoints:
1. `docs/specs/fullbleed.audit_registry.v1.yaml` (source-of-truth spec artifact)
2. `crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml` (embedded runtime contract)

Implication:
- Rule additions are not only code changes; they are contract changes.
- Contract fingerprint changes are expected and must be treated as intentional release deltas.

### 3.7 Heuristic Design Theory (for this audit)
1. Figure alt budget:
   - Compute normalized visible-length for `alt`.
   - Default threshold: `150` chars (profile override allowed).
   - Result: `warn` when over budget (strict profile may elevate).
2. Alt/caption redundancy:
   - Normalize both strings (casefold, collapse whitespace/punctuation).
   - Compute token overlap/similarity score.
   - Warn above similarity threshold (start conservative to avoid false positives).
3. DL fragmentation:
   - Detect adjacent/sibling `dl` groups within same semantic container.
   - Flag repeated micro-lists that form one logical field region.
4. Redundant ARIA:
   - Detect ARIA role/state duplicating implicit native semantics.
   - Keep allowlist for legitimate overrides; default to `warn`.

### 3.8 Observability-First Theory
Every new rule should emit:
1. rule-level evidence (selector, values, score/threshold)
2. aggregate counters in run metrics
3. profile + mode behavior that can be CI-gated

## 4. Code Breadcrumbs (Exact Touchpoints)

Primary implementation files:
1. `python/fullbleed/audit_prototype.py`
2. `docs/specs/fullbleed.audit_registry.v1.yaml`
3. `crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml`
4. `src/lib.rs`
5. `src/python.rs`
6. `python/fullbleed/accessibility/engine.py`

Primary tests to extend:
1. `tests/test_accessibility_audit_prototype.py`
2. `tests/test_fullbleed_engine_accessibility_verifier.py`
3. `tests/test_fullbleed_engine_pmr.py`
4. `tests/test_accessibility_audit_specs.py`
5. `tests/test_audit_contract_runtime.py`

Canary/remediation corpus targets:
1. `python/fullbleed/ui/cav/*.py` (affected CAVs)
2. audited 5-doc outputs + sidecars in your working corpus path

## 5. Sprint Breadcrumbs

## B0 - Baseline Capture

Deliverables:
- Capture current verifier outputs for the five audited docs.
- Add `audit_baseline/` snapshots of:
  - a11y verify JSON
  - PMR JSON
  - emitted HTML

Gate:
- Baseline artifacts exist for all 5 docs.

Execution snapshot (March 3, 2026):
1. Manifest: `audit_baseline/audited_docs.v1.json`
2. Capture runner: `tools/capture_audit_baseline.py`
3. Artifacts: `audit_baseline/third_party_2026_03/`
4. Summary: `audit_baseline/third_party_2026_03/baseline_summary.md`

Repeatable command:
`.\.venv\Scripts\python.exe tools\capture_audit_baseline.py`

Optional diff command (rule-family deltas vs previous capture):
`.\.venv\Scripts\python.exe tools\capture_audit_baseline.py --compare-summary <prior-baseline-summary.json>`

Code breadcrumbs:
1. Use `python/fullbleed/accessibility/engine.py` `render_bundle(...)` output JSONs as baseline capture source.
2. Persist verifier/PMR/HTML artifacts under `audit_baseline/` with deterministic names.
3. Extend CI helper script (or add one) to diff warning counts by rule family.

## B1 - Figure Verbosity + Duplication Rules

Engine/verifier additions:
- `fb.a11y.figure.alt_length_budget_seed`
  - warn if `alt` length exceeds default budget (150 chars; configurable).
- `fb.a11y.figure.caption_redundancy_seed`
  - warn when `alt` and `figcaption` are near-duplicate (normalized text similarity threshold).
- `fb.a11y.figure.missing_effective_text_seed`
  - fail when informative figure lacks both useful `alt` and meaningful caption.

UI/accessibility authoring updates:
- Document guidance: concise `alt`, complementary caption.
- Expose optional policy knobs (strict profiles can elevate warnings to errors).

Gate:
- Rules emitted in verifier output with evidence payloads (`alt_len`, similarity score, figure selector).

Execution snapshot (March 3, 2026):
1. Implemented in:
   - `python/fullbleed/audit_prototype.py`
   - `src/lib.rs`
   - `docs/specs/fullbleed.audit_registry.v1.yaml`
   - `crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml`
2. Added coverage tests:
   - `tests/test_accessibility_audit_prototype.py::test_prototype_emits_figure_alt_budget_redundancy_and_effective_text_rules`
   - `tests/test_fullbleed_engine_accessibility_verifier.py::test_engine_verifier_emits_figure_alt_budget_redundancy_and_effective_text_rules`
3. Verified with:
   - `.\.venv\Scripts\python.exe -m pytest tests/test_accessibility_audit_prototype.py tests/test_fullbleed_engine_accessibility_verifier.py tests/test_accessibility_audit_specs.py tests/test_audit_contract_runtime.py -q`

Code breadcrumbs:
1. `python/fullbleed/audit_prototype.py`
2. Add `HtmlFacts` fields for figure/caption text collection and budget counters.
3. Add parser capture in `_P._tag(...)` + end-tag handlers for `figure`, `img`, `figcaption`.
4. Emit findings in `prototype_verify_accessibility(...)`.
5. Mirror PMR advisory audits in `prototype_verify_paged_media_rank(...)` when useful.
6. Add rule entries in both registry files.
7. Add schema-safe evidence payload assertions in:
   - `tests/test_accessibility_audit_prototype.py`
   - `tests/test_fullbleed_engine_accessibility_verifier.py`

## B2 - Description List Consolidation Rules

Verifier additions:
- `fb.a11y.dl.fragmentation_seed`
  - warn when adjacent `dl` siblings represent a single logical field group and should be merged.
- `fb.a11y.dl.group_consistency_seed`
  - warn on repeated tiny `dl` blocks with identical container semantics/class lineage.

Authoring surface updates:
- Strengthen `FieldGrid` docs/examples toward one unified list per logical region.
- Optional helper transform in authoring layer to coalesce contiguous field groups.

Gate:
- Fragmentation warnings visible in verifier; remediated canary docs show reduced/zero findings.

Execution snapshot (March 3, 2026):
1. Implemented rules:
   - `fb.a11y.dl.fragmentation_seed`
   - `fb.a11y.dl.group_consistency_seed`
2. Implemented in:
   - `python/fullbleed/audit_prototype.py`
   - `src/lib.rs`
   - `docs/specs/fullbleed.audit_registry.v1.yaml`
   - `crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml`
3. Added coverage tests:
   - `tests/test_accessibility_audit_prototype.py::test_prototype_emits_dl_fragmentation_and_group_consistency_rules`
   - `tests/test_fullbleed_engine_accessibility_verifier.py::test_engine_verifier_emits_dl_fragmentation_and_group_consistency_rules`
4. Verified with:
   - `.\.venv\Scripts\python.exe -m pytest tests/test_accessibility_audit_prototype.py tests/test_fullbleed_engine_accessibility_verifier.py tests/test_accessibility_audit_specs.py tests/test_audit_contract_runtime.py -q`

Code breadcrumbs:
1. `python/fullbleed/audit_prototype.py`
2. Add `HtmlFacts` counters for adjacent `dl` clusters + repeated micro-lists.
3. Parser heuristics: same parent/container lineage + short dt/dd groups -> fragmentation signal.
4. Emit `fb.a11y.dl.fragmentation_seed` and `fb.a11y.dl.group_consistency_seed`.
5. Update CAV builders using repeated local `FieldGrid(...)` blocks:
   - `python/fullbleed/ui/cav/redaction_request_form.py`
   - `python/fullbleed/ui/cav/warranty_deed.py`
   - `python/fullbleed/ui/cav/recorded_plat.py`
6. Add regression tests in `tests/test_fullbleed_ui_cav.py` for consolidated output shape.

## B3 - ARIA Hygiene (Native-First Enforcement)

Verifier additions:
- `fb.a11y.aria.redundant_role_native_seed`
  - warn when explicit role duplicates implicit semantics (example: `role="navigation"` on `nav`).
- `fb.a11y.aria.redundant_state_native_seed`
  - warn on ARIA states/properties with no additional semantic value in context.

Policy:
- Strict profile option to escalate selected redundant ARIA warnings to fail.

Gate:
- Rule coverage added to machine-readable report.
- Canary docs pass with zero high-severity ARIA hygiene findings.

Execution snapshot (March 3, 2026):
1. Implemented rules:
   - `fb.a11y.aria.redundant_role_native_seed`
   - `fb.a11y.aria.redundant_state_native_seed`
2. Implemented in:
   - `python/fullbleed/audit_prototype.py`
   - `src/lib.rs`
   - `docs/specs/fullbleed.audit_registry.v1.yaml`
   - `crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml`
3. Added coverage tests:
   - `tests/test_accessibility_audit_prototype.py::test_prototype_emits_redundant_aria_native_rules`
   - `tests/test_fullbleed_engine_accessibility_verifier.py::test_engine_verifier_emits_redundant_aria_native_rules`
4. Verified with:
   - `.\.venv\Scripts\python.exe -m pytest tests/test_accessibility_audit_prototype.py tests/test_fullbleed_engine_accessibility_verifier.py tests/test_accessibility_audit_specs.py tests/test_audit_contract_runtime.py -q`

Code breadcrumbs:
1. `python/fullbleed/audit_prototype.py`:
   - detect redundant role/native pairs (`nav`, `main`, `table`, etc.).
   - detect non-value-added ARIA state usage.
2. Add rule entries to both registry files.
3. Add engine parity checks:
   - `tests/test_fullbleed_engine_accessibility_verifier.py`
4. Keep explicit allowlist in code comments/tests for acceptable exceptions.

## B4 - CI Gates + Observability Contract

Add report metrics:
- `figure_alt_over_budget_count`
- `figure_caption_redundancy_count`
- `dl_fragmentation_count`
- `redundant_aria_count`

Gate policy (`strict`):
- Any fail in these families blocks.
- Warnings budget configurable; default target for audited corpus: zero.

Artifacts:
- Include counts in run report metrics and PMR/a11y sidecars.

Execution snapshot (March 3, 2026):
1. Added observability counters:
   - `figure_alt_over_budget_count`
   - `figure_caption_redundancy_count`
   - `dl_fragmentation_count`
   - `redundant_aria_count`
2. Wired in:
   - `python/fullbleed/audit_prototype.py`
   - `src/python.rs`
   - `docs/specs/fullbleed.a11y.verify.v1.schema.json`
   - `docs/specs/fullbleed.pmr.v1.schema.json`
3. Validation updates:
   - `tests/test_accessibility_audit_prototype.py`
   - `tests/test_fullbleed_engine_accessibility_verifier.py`
   - `tests/test_fullbleed_engine_pmr.py`
4. Verified with:
   - `.\.venv\Scripts\python.exe -m pytest tests/test_accessibility_audit_prototype.py tests/test_fullbleed_engine_accessibility_verifier.py tests/test_fullbleed_engine_pmr.py tests/test_accessibility_audit_specs.py tests/test_audit_contract_runtime.py -q`

Code breadcrumbs:
1. `python/fullbleed/audit_prototype.py`:
   - extend `coverage/observability` payload with new aggregate counters.
2. `src/python.rs`:
   - ensure native report serialization surfaces the same counters.
3. `python/fullbleed/accessibility/engine.py`:
   - carry counters into `render_bundle(...)` run report metrics.
4. `tests/test_accessibility_audit_specs.py`:
   - validate schema compatibility and presence of counters.

## B5 - Corpus Remediation Pass (the 5 audited docs)

Remediate documents using new rules/guidance:
- shorten long `alt`,
- de-duplicate caption content,
- merge fragmented description lists,
- remove unnecessary ARIA.

Gate:
- Re-run verifier + PMR on all 5 docs.
- No fail findings in new rule families.
- Warning counts materially reduced from B0 baseline.

Execution snapshot (March 3, 2026):
1. Re-ran baseline capture with current verifier:
   - `.\.venv\Scripts\python.exe tools\capture_audit_baseline.py`
2. New-rule family outcomes across all 5 audited docs:
   - `fb.a11y.figure.alt_length_budget_seed`: `not_applicable`
   - `fb.a11y.figure.caption_redundancy_seed`: `not_applicable`
   - `fb.a11y.figure.missing_effective_text_seed`: `not_applicable`
   - `fb.a11y.dl.fragmentation_seed`: `pass`
   - `fb.a11y.dl.group_consistency_seed`: `pass`
   - `fb.a11y.aria.redundant_role_native_seed`: `pass`
   - `fb.a11y.aria.redundant_state_native_seed`: `pass`
3. Gate check:
   - No `fail` verdicts in new B1/B2/B3 rule families for audited corpus.

Code breadcrumbs:
1. CAV/doc authoring surfaces:
   - `python/fullbleed/ui/cav/*.py`
   - scaffold-derived docs under your `_accessibility_examples` working set.
2. Validate with:
   - engine verifier (`verify_accessibility_artifacts`)
   - PMR (`verify_paged_media_rank_artifacts`)
   - run bundle sidecars.

## B6 - NVDA Spot Validation Loop

Manual confirmation (short script-based protocol):
- NVDA linear read-through for each remediated doc.
- Verify reduced repetition and smoother section transitions.
- Record concise findings in `audit_baseline/nvda_spot_check.md`.

Gate:
- Spot check completed for all 5 docs with no blocker regressions.

Execution status (March 4, 2026):
1. Protocol scaffold created:
   - `audit_baseline/nvda_spot_check.md`
2. Current state:
   - checklist prepared for all 5 audited docs,
   - live NVDA session waived by product decision,
   - sprint acceptance based on machine-verifiable coverage of all third-party finding families.

Code breadcrumbs:
1. Save NVDA observation output in `audit_baseline/nvda_spot_check.md`.
2. Link each observation to rule IDs and HTML artifact path for traceability.

## 6. Proposed Rule IDs / Contract Additions

1. `fb.a11y.figure.alt_length_budget_seed` (warn/default)
2. `fb.a11y.figure.caption_redundancy_seed` (warn/default)
3. `fb.a11y.figure.missing_effective_text_seed` (fail/default)
4. `fb.a11y.dl.fragmentation_seed` (warn/default)
5. `fb.a11y.dl.group_consistency_seed` (warn/default)
6. `fb.a11y.aria.redundant_role_native_seed` (warn/default)
7. `fb.a11y.aria.redundant_state_native_seed` (warn/default)

Note: Keep these as optimization/quality rules in v1 unless strict mode explicitly promotes severity.

## 7. Acceptance Criteria

1. All 7 proposed rules implemented and emitted in verifier JSON.
2. Run reports expose the four new aggregate counters.
3. Five audited docs remediated with measurable warning reduction.
4. No new blockers introduced in existing WCAG/Section 508 gates.
5. NVDA spot validation confirms improved usability (less duplicate speech, clearer grouping).

## 8. Risks and Mitigations

Risk: false positives in caption-duplication detection.  
Mitigation: start as `warn`, expose similarity score/evidence, tune threshold with corpus.

Risk: over-aggressive DL consolidation heuristics.  
Mitigation: treat as advisory warning first; require structural evidence in rule payload.

Risk: ARIA rule overreach on legitimate exceptions.  
Mitigation: maintain allowlist for valid overrides and emit rationale in evidence.

## 9. Execution Order (Recommended)

1. B0 baseline capture
2. B1 figure rules
3. B3 ARIA hygiene rules
4. B2 DL fragmentation rules
5. B4 CI/observability wiring
6. B5 corpus remediation
7. B6 NVDA spot validation and closeout

## 10. Out of Scope (This Sprint)

1. Full visual redesign of existing CAV outputs.
2. New document kit families unrelated to audited findings.
3. PDF/UA deep tag-graph expansion not tied to these HTML-level findings.
