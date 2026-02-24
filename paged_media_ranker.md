# Paged Media Ranker (Lighthouse-Style) - Spec + Feasibility

## Purpose

Specify a FullBleed-native, Lighthouse-style ranker for paged media outputs (HTML/CSS -> paged render), with:

- weighted categories and audits
- deterministic engine-native scoring
- fail-fast gating
- rich diagnostics/opportunities
- explicit separation from legal accessibility conformance claims

This is a companion to `accessibility_verifier.md` and focuses on **paged-media-specific quality and accessibility readiness**.

## Executive Verdict

Feasible and strategically strong.

FullBleed is unusually well-positioned to do this because the engine owns:

- pagination
- layout planning
- render metrics
- known-loss diagnostics
- overflow/placement signals
- authoring and emitted artifact paths

That means we can build a ranker that is more deterministic and more useful for paged media than Lighthouse is for browser pages, while still borrowing the best parts of the Lighthouse UX model.

## What "Lighthouse-Style" Should Mean Here

Borrow:

- categories with weights
- audit cards (pass/fail/warn + evidence)
- opportunities/diagnostics surfaced separately from hard failures
- CI assertions and fail levels (`off|warn|error`)
- trend-friendly numeric score

Do not borrow:

- single-score conformance claims
- browser-only audit assumptions
- viewport-centric heuristics that ignore pagination realities

## Problem We Are Solving

We need a machine-verifiable, observable way to assert HTML accessibility and paged-media correctness for FullBleed outputs. Current state has improving semantics but lacks:

- standardized score/rank for regression tracking
- engine-native paged-media quality checks
- consistent audit evidence format
- category-based fail-fast profiles

For CAVs and transactional documents, "paged-media correctness" is part of accessibility readiness because same-use parity depends on:

- stable pagination
- preserved reading order across pages
- field/table integrity across page breaks
- no clipped or hidden content

## Scope Boundary (Important)

This ranker is **not** a legal Section 508 conformance engine.

It is a **paged-media compatibility ranker** and diagnostics system that can incorporate accessibility checks.

Output model should include:

- `paged_media_rank` (0-100)
- `a11y_machine_subset_status`
- `manual_review_debt`
- `gate_result`

## Proposed Product Name (Working)

Use a neutral internal name first:

- `PMR` = Paged Media Ranker

Alternative UX names (later):

- `Beacon`
- `Signal`
- `PrintLight` (probably too on-the-nose)

## Core Design: Categories + Audits + Evidence

## Top-Level Categories (v1)

Weighted to reflect paged-media utility and accessibility readiness.

1. `Document Semantics` (20)
- metadata and structural semantics needed for a11y and downstream tagging

2. `Reading Order & Structure` (20)
- sequence integrity within and across pages

3. `Paged Layout Integrity` (25)
- overflow, clipping, known-loss, break behavior

4. `Field/Table/Form Integrity` (20)
- same-use structures for transactional docs / CAVs

5. `Artifact Packaging & Reproducibility` (15)
- emitted artifacts, metadata persistence, hashes, deterministic evidence

Total: 100

Why no separate "Accessibility" category in v1:

- accessibility is distributed across semantics, reading order, and field/table integrity in paged media
- avoids a misleading single category while coverage is still expanding

We can add `Browser Runtime Accessibility` as a v2 category when Lighthouse/axe adapters are integrated.

## Audit Types

Each audit has:

- `audit_id` (stable)
- `category`
- `weight`
- `verification_mode` (`machine|manual|hybrid`)
- `severity`
- `verdict` (`pass|fail|warn|not_applicable|manual_needed`)
- `evidence`
- `fix_hint`

Audit classes:

- `Required`: can fail gate directly
- `Scored`: affects numeric rank
- `Diagnostic`: reported, no score impact (initially)
- `Opportunity`: optimization/improvement suggestion (non-blocking)

## Scoring Model (Lighthouse-Style, Paged-Media Native)

## Audit Score

For v1, keep it simple and predictable:

- `pass` = 1.0
- `warn` = 0.5 (only for scored audits that permit warn)
- `fail` = 0.0
- `not_applicable` = excluded from denominator
- `manual_needed` = excluded from score but counted as manual debt (or optionally partial penalty via confidence adjustment)

## Category Score

Weighted average of scored audits in the category.

If a category has too many `manual_needed` or `not_evaluated` audits, attach low confidence and optionally cap category score display confidence.

## Overall Paged Media Rank

Weighted average of category scores (weights above), then:

- apply confidence adjustment (optional v1.1)
- report raw score and confidence separately

Recommended outputs:

- `rank.score` (0-100)
- `rank.confidence` (0-100)
- `rank.band` (`excellent|good|watch|poor`)

## Gate Model (Fail-Fast)

Independent of score.

Hard-fail examples:

- missing document title
- missing/invalid document language
- duplicate IDs
- broken ARIA references
- overflow/clipping detected in strict profile
- critical known-loss present (profile-dependent)
- page-count parity target violated (CAV profile)
- field/table split integrity violation (when marked non-splittable)

This avoids "high score but broken output" failure modes.

## Paged-Media-Specific Categories and Candidate Audits (v1)

## 1) Document Semantics (20)

Purpose: document-level metadata and structure survive emission and remain machine-usable.

Candidate audits:

- `pmr.doc.html_wrapper_present`
  - HTML artifact is document-wrapped (doctype/html/head/body)
- `pmr.doc.lang_present_valid`
  - `<html lang>` present and non-empty (basic format validation)
- `pmr.doc.title_present_nonempty`
  - `<title>` present and non-empty
- `pmr.doc.single_main`
  - one primary content root (`main`) when applicable
- `pmr.doc.metadata_engine_persistence`
  - engine metadata (`document_lang`, `document_title`) preserved into emitted HTML

Feasibility:

- High (already partly implemented/tested)

## 2) Reading Order & Structure (20)

Purpose: paged output remains semantically navigable and sequence-correct.

Candidate audits:

- `pmr.structure.heading_nonempty`
- `pmr.structure.heading_level_jumps` (warn by default)
- `pmr.structure.landmark_labeling`
- `pmr.structure.dom_order_stable`
  - emitted DOM order is the declared reading order contract
- `pmr.structure.cross_page_sequence_integrity`
  - sequence content not reordered by pagination pipeline (engine evidence + structure anchors)

Feasibility:

- High for DOM/static checks
- Medium for cross-page structure anchors (needs instrumentation)

## 3) Paged Layout Integrity (25)

Purpose: no hidden or broken content due to pagination/layout.

Candidate audits:

- `pmr.layout.overflow_none`
  - no overflow placements detected in diagnostics
- `pmr.layout.known_loss_none_critical`
  - no critical known-loss events (`jit.known_loss`) under selected profile
- `pmr.layout.page_count_target`
  - meets target parity/range when specified (e.g., CAV same-use)
- `pmr.layout.clip_risk_none`
  - no clipping-risk patterns for required-visible content (engine + style hints)
- `pmr.layout.lazy_convergence`
  - lazy layout converged (if lazy strategy enabled)
- `pmr.layout.header_footer_consistency`
  - repeated header/footer render plan consistency when configured

Feasibility:

- High for overflow/known-loss/page-count
- Medium for clip-risk and consistency heuristics

Engine signals already available:

- overflow/known-loss JIT diagnostics
- `validate_component_mount(...)` summaries
- `DocumentMetrics` / `PageMetrics`

## 4) Field/Table/Form Integrity (20)

Purpose: preserve same-use transactional semantics across page rendering.

Candidate audits:

- `pmr.forms.id_ref_integrity`
  - duplicate IDs / ARIA refs / label refs
- `pmr.fields.fieldgrid_contract`
  - `FieldGrid` emits semantic pairs correctly (if used)
- `pmr.tables.semantic_table_headers`
  - `th`/`td`, caption, scope checks
- `pmr.tables.row_split_integrity`
  - rows/critical grouped fields not split incorrectly (profile-driven)
- `pmr.signatures.text_semantics_present`
  - signatures represented textually (status/name/date/method where modeled)
- `pmr.seals.textual_presence`
  - seal/stamp presence represented textually when source semantics require it

Feasibility:

- High for DOM/static checks
- Medium for split-integrity (requires mapping rendered flowables back to semantic blocks)

## 5) Artifact Packaging & Reproducibility (15)

Purpose: outputs are testable and comparable.

Candidate audits:

- `pmr.artifacts.html_emitted`
- `pmr.artifacts.css_emitted`
- `pmr.artifacts.html_css_hash_recorded`
- `pmr.artifacts.engine_version_recorded`
- `pmr.artifacts.metadata_reported`
- `pmr.artifacts.linked_css_reference` (v1 warn / v2 pass requirement if enabled)

Feasibility:

- High

Note:

- We currently emit HTML and CSS separately but do not auto-inject `<link rel="stylesheet">` in emitted HTML. This should be tracked as a packaging audit, not hidden.

## CAV-Specific Profile (Paged Same-Use Rank)

A `cav` profile should extend PMR with same-use parity checks.

Additional audits:

- `pmr.cav.document_only_content`
  - no remediation notes/review annotations in deliverable body
- `pmr.cav.section_coverage_required`
  - required source sections represented (via parity checklist sidecar)
- `pmr.cav.pagination_parity_target`
  - exact page parity or declared tolerance
- `pmr.cav.signature_fields_inline`
  - signatures represented in-place, not only inventory summary
- `pmr.cav.uncertain_text_marked`
  - illegible/uncertain transcriptions marked transparently (no invented content)

This profile is directly aligned to the CAV workflow we established.

## Evidence and Diagnostics (Observability)

## Audit Evidence Requirements

Every non-pass audit must include at least one of:

- CSS selector
- DOM path
- semantic path (e.g., `field:20c`)
- page number(s)
- render-plan node id (future)
- JIT diagnostic record reference

Useful evidence payloads:

- offending HTML snippet (trimmed)
- computed/observed values
- page indices
- bounding boxes (future)
- suggested fix text

## "Opportunities" UX (Lighthouse-like, but for paged media)

Examples:

- "Reduce boilerplate font-size variance to improve page-count stability"
- "Mark signature witness rows as keep-together to avoid split risk"
- "Add CSS link injection for standalone HTML artifact review"
- "Add table caption for semantic table in certificate section"

These are not hard fails, but improve rank and maintainability.

## Data Model (PMR JSON Schema Sketch)

Top-level:

- `schema`
- `target`
- `profile`
- `rank`
- `gate`
- `categories`
- `audits`
- `manual_debt`
- `coverage`
- `tooling`
- `artifacts`
- `baseline_diff` (optional)

Example skeleton:

```json
{
  "schema": "fullbleed.pmr.v1",
  "profile": "cav",
  "rank": {
    "score": 92.0,
    "confidence": 88.0,
    "band": "good"
  },
  "gate": {
    "ok": true,
    "mode": "error",
    "error_count": 0,
    "warn_count": 3
  },
  "categories": [
    {"id": "document-semantics", "score": 100, "weight": 20},
    {"id": "paged-layout-integrity", "score": 86, "weight": 25}
  ],
  "audits": [
    {
      "audit_id": "pmr.doc.lang_present_valid",
      "category": "document-semantics",
      "weight": 3,
      "verdict": "pass",
      "verification_mode": "machine",
      "severity": "high",
      "evidence": [{"selector": "html", "value": "en-US"}]
    }
  ]
}
```

## Feasibility by Implementation Layer

## Layer 1: Engine-Native PMR Core (High Feasibility)

Inputs available now or nearly now:

- emitted HTML/CSS artifacts
- engine metadata (`document_lang`, `document_title`)
- `validate_component_mount` signals (overflow, CSS warnings, known-loss)
- render metrics (`DocumentMetrics`, `PageMetrics`)
- `A11yContract` reports
- CAV parity sidecars

Value:

- immediate fail-fast gating
- deterministic scores
- no browser dependency

## Layer 2: Render-Plan Anchors / Box Mapping (Medium Feasibility)

Needed for stronger paged-media audits:

- semantic node -> rendered page/box mapping
- split detection for rows/field groups
- cross-page sequence checks with stronger evidence

This is the biggest engine instrumentation investment, but it unlocks truly paged-native ranking.

## Layer 3: Browser Adapter Augmentation (Medium Feasibility)

Optional category:

- `browser-runtime-accessibility`

Use Lighthouse/axe audits as supplemental evidence.

Keep separate from PMR core so engine score remains deterministic and fast.

## How This Integrates With Current FullBleed Workflow

## Authoring Stage

- `A11yContract` checks semantics pre-render
- PMR can ingest these as pre-render audits (same rule namespace where possible)

## Emission Stage

- engine emits HTML/CSS artifacts (now implemented)
- PMR verifies doc-level semantics persisted at artifact boundary

## Render Stage

- PMR consumes engine diagnostics/metrics
- checks paged layout integrity (overflow, known loss, page count targets)

## CAV Stage

- PMR ingests parity sidecar and applies `cav` profile checks
- enforces same-use expectations as machine-verifiable policy

## Proposed CLI/API (Sketch)

CLI:

```bash
fullbleed pmr verify \
  --html output/doc.html \
  --css output/doc.css \
  --profile cav \
  --mode error \
  --parity-report output/doc_parity_report.json \
  --component-validation output/doc_component_mount_validation.json \
  --a11y-report output/doc_a11y_validation.json \
  --out output/doc_pmr.json
```

Python:

```python
pmr = fullbleed.verify_paged_media_rank(
    html_path="output/doc.html",
    css_path="output/doc.css",
    profile="cav",
    mode="error",
    a11y_report=a11y_report,
    component_validation=component_validation,
    parity_report=parity_report,
)
```

## Breadcrumbs (Implementation Plan)

## Breadcrumb 0: PMR Spec Freeze

Deliverables:

- PMR category list + weights
- audit ID namespace (`pmr.*`)
- gate policy and profiles (`strict`, `cav`, `transactional`)
- JSON schema draft

Exit criteria:

- team agrees on score semantics and hard gates
- team agrees "score != conformance"

## Breadcrumb 1: PMR Aggregator (Python First, Fastest Path)

Implement a Python-side PMR aggregator using existing artifacts/sidecars:

- `A11yContract` report
- component mount validation
- CAV parity report
- HTML/CSS artifact checks

Why first:

- immediate value
- no Rust changes required
- validates score model before engine API hardening

Exit criteria:

- stable PMR JSON on current CAV sample
- deterministic score + gate

## Breadcrumb 2: Engine-Native PMR Core (Rust)

Move high-confidence checks into engine-native verifier/ranker:

- metadata audits
- artifact packaging audits
- overflow/known-loss/layout metrics audits
- page-count target checks

Exit criteria:

- `fullbleed` Python API exposes PMR run
- same score for equivalent input vs Python prototype (for overlapping audits)

## Breadcrumb 3: Rule Namespace Unification

Unify PMR + accessibility verifier IDs where applicable:

- `fb.a11y.*` for standards/convention checks
- `pmr.*` for paged-media quality/rank checks
- cross-links between related audits

Exit criteria:

- no duplicate issue reporting without relationship metadata

## Breadcrumb 4: Render-Plan Anchor Instrumentation

Add semantic-to-render anchor mapping for stronger paged audits:

- split detection
- page-local evidence
- row/field group integrity checks

Exit criteria:

- PMR can identify specific page and structure for split violations

## Breadcrumb 5: Browser Adapter Augmentation (Optional v2)

Integrate Lighthouse/axe as supplemental category/evidence:

- version-pinned
- audit normalization
- raw outputs preserved

Exit criteria:

- PMR report includes optional runtime category without changing engine-core determinism

## Breadcrumb 6: Baselines + Regressions + Goldens

Add PMR baseline compare and CI gating:

- score delta
- new hard fails
- new warnings
- confidence delta

Exit criteria:

- canary samples and marriage CAV covered

## Initial Audit Set Recommendation (Start Small, High Signal)

Ship these first:

- `pmr.doc.lang_present_valid`
- `pmr.doc.title_present_nonempty`
- `pmr.doc.metadata_engine_persistence`
- `pmr.layout.overflow_none`
- `pmr.layout.known_loss_none_critical`
- `pmr.layout.page_count_target` (when target supplied)
- `pmr.forms.id_ref_integrity`
- `pmr.tables.semantic_table_headers`
- `pmr.signatures.text_semantics_present` (profile-driven)
- `pmr.cav.document_only_content` (CAV profile)

This set already creates a meaningful, trustworthy ranker.

## Risks and Mitigations

## Risk: Score gaming

Mitigation:

- hard gate independent of score
- confidence and manual debt shown alongside score

## Risk: Too many audits too early (noise)

Mitigation:

- ship v1 with small high-confidence audit set
- use diagnostic/opportunity audits for non-blocking signals first

## Risk: Paged-media heuristics become opaque

Mitigation:

- evidence-first audit design
- per-audit fix hints
- deterministic thresholds and profile docs

## Recommendation

Proceed with PMR as a first-class companion to the accessibility verifier.

Implement the scoring and aggregation model first (Python prototype over existing sidecars), then migrate the deterministic/high-signal audits into an engine-native PMR core. This gives immediate observability and a path to a genuinely differentiated paged-media quality signal.

