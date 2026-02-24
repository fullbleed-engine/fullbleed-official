# PDF/UA Output Sprint (Accessibility Stack Surface)

## Status

Planning sprint (implementation-ready working document).

## Why This Sprint

We have a strong HTML accessibility authoring and audit foundation, but the product boundary is still unclear for PDF accessibility output:

- `fullbleed.ui.accessibility` exists for semantic authoring
- engine-level `pdf_profile` supports `"tagged"` and currently aliases `"pdfua"` to the same profile path
- there is no clear top-level API surface that says "this is the accessibility stack" and enforces accessibility defaults

This sprint defines and ships that boundary.

## Goal

Create a dedicated `fullbleed.accessibility` surface that:

1. is the canonical accessibility-first API entrypoint
2. always drives the engine in PDF/UA-targeted mode (no accidental non-accessible PDF profile from this surface)
3. makes accessibility verification/observability first-class in the render pipeline
4. reduces confusion between:
   - semantic authoring (`fullbleed.ui.accessibility`)
   - audit/reporting (`a11y verifier`, PMR)
   - accessible PDF output mode (PDF/UA-targeted engine path)

## Non-Goals (This Sprint)

- Full PDF/UA conformance automation/certification
- Full PDF artifact semantic verifier parity with HTML verifier
- Replacing `PdfEngine` for non-accessibility workflows
- Expanding WCAG breadth further (backlogged after current sprint)

## Process Notes (From Current Epic)

These are now hard lessons and should shape this sprint.

- Lock claim language before adding scores/flags.
- Keep the audit contract immutable per build (contract crate remains authority).
- Separate "targeted mode" from "conformance claim" in both API and reports.
- Keep CAV deliverables document-only; evidence stays in sidecars/reports.
- Prefer sequential rebuild/test on Windows (`maturin develop` then pytest), not parallel, to avoid stale extension loads during rebuild.

## Key Current Ambiguity (Must Resolve)

Today Python accepts `pdf_profile="pdfua"` but maps it to the same tagged profile path:

- `src/python.rs:998`

That is a useful compatibility alias, but not a sufficient product boundary. Users can still call the generic engine and miss accessibility expectations (metadata, verification, report emission, fail-fast defaults).

## Product Direction (Locked for Sprint Planning)

### Accessibility Stack Layout

Introduce a new top-level Python package:

- `fullbleed.accessibility`

This package is the canonical surface for accessibility workflows and composes:

- semantic authoring (`fullbleed.ui.accessibility`)
- accessibility-first engine wrapper(s)
- verifier / PMR integration
- accessibility-targeted artifact emission helpers

### Clear Surface Separation

- `fullbleed.ui.accessibility`
  - authoring primitives and semantic composition
- `fullbleed.accessibility`
  - accessibility workflow/runtime surface
  - engine wrapper + verification defaults + reports + policy

This avoids overloading `ui.accessibility` with runtime/render/audit concerns.

## Proposed API (V1)

### Python (`fullbleed.accessibility`)

- `AccessibilityEngine(...)`
  - wraps `fullbleed.PdfEngine`
  - forces PDF/UA-targeted profile internally
  - requires/accessibly defaults document metadata (`lang`, `title`)
  - exposes `verify_*` and `render_*` methods with accessibility-first defaults

- `render_accessible_document(...)`
  - high-level helper for HTML/CSS/PDF + audit sidecars
  - intended for CAV and scaffold workflows

- `AccessibilityRunResult`
  - normalized paths + verifier/PMR results + contract fingerprint + profile metadata

### Authoring Integration

`fullbleed.accessibility` should accept `fullbleed.ui.accessibility` artifacts/components directly, but should not duplicate authoring primitives.

## Implementation Scaffolding (Execution-Ready)

This section is intentionally concrete so implementation can proceed with minimal design churn.

## Package/Module Scaffold (Python)

Create:

- `python/fullbleed/accessibility/__init__.py`
- `python/fullbleed/accessibility/engine.py`
- `python/fullbleed/accessibility/runtime.py`
- `python/fullbleed/accessibility/types.py` (optional but recommended)

Recommended responsibilities:

- `__init__.py`
  - exports public symbols only (`AccessibilityEngine`, `render_accessible_document`, `AccessibilityRunResult`, config/result dataclasses)
  - no business logic

- `engine.py`
  - `AccessibilityEngine` wrapper around `fullbleed.PdfEngine`
  - policy enforcement (forced PDF/UA-targeted mode)
  - render/emit methods
  - verifier/PMR orchestration entrypoints

- `runtime.py`
  - bundle emission orchestration
  - path normalization and sidecar writing
  - run-report composition helpers
  - backward-compatible argument normalization helpers

- `types.py`
  - dataclasses / typed dicts for config and results
  - minimizes signature bloat in `engine.py`

## Public API Skeleton (Recommended Signatures)

These signatures are intentionally stable/minimal for sprint execution.

```python
# python/fullbleed/accessibility/engine.py

class AccessibilityEngine:
    def __init__(
        self,
        *,
        page_size: str = "LETTER",
        document_lang: str | None = None,
        document_title: str | None = None,
        strict: bool = False,
        emit_reports_by_default: bool = True,
        render_previews_by_default: bool = True,
        # intentionally no pdf_profile argument here
        **engine_kwargs,
    ) -> None: ...

    @property
    def document_lang(self) -> str | None: ...

    @document_lang.setter
    def document_lang(self, value: str | None) -> None: ...

    @property
    def document_title(self) -> str | None: ...

    @document_title.setter
    def document_title(self, value: str | None) -> None: ...

    def document_metadata(self) -> dict[str, str | None]: ...

    def emit_html(self, body_html: str, out_html_path: str, *, css_href: str | None = None) -> str: ...
    def emit_css(self, css_text: str, out_css_path: str) -> str: ...
    def emit_artifacts(self, body_html: str, css_text: str, out_html_path: str, out_css_path: str) -> tuple[str, str]: ...

    def verify_accessibility_artifacts(
        self,
        html_path: str,
        css_path: str,
        *,
        profile: str = "cav",
        mode: str = "error",
        a11y_report: dict | None = None,
        claim_evidence: dict | None = None,
        render_preview_png_path: str | None = None,
    ) -> dict: ...

    def verify_pmr_artifacts(
        self,
        html_path: str,
        css_path: str,
        *,
        profile: str = "cav",
        mode: str = "error",
        component_validation: dict | None = None,
        parity_report: dict | None = None,
        run_report: dict | None = None,
    ) -> dict: ...

    def verify_pdf_ua_seed_artifacts(
        self,
        pdf_path: str,
        *,
        mode: str = "error",
    ) -> dict: ...

    def export_reading_order_trace(
        self,
        pdf_path: str,
        *,
        out_path: str | None = None,
    ) -> dict: ...

    def export_pdf_structure_trace(
        self,
        pdf_path: str,
        *,
        out_path: str | None = None,
    ) -> dict: ...

    def render_bundle(
        self,
        *,
        body_html: str,
        css_text: str,
        out_dir: str,
        stem: str,
        profile: str = "cav",
        a11y_mode: str | None = "raise",
        a11y_report: dict | None = None,
        claim_evidence: dict | None = None,
        component_validation: dict | None = None,
        parity_report: dict | None = None,
        source_analysis: dict | None = None,
        render_preview_png: bool | None = None,
        run_verifier: bool | None = None,
        run_pmr: bool | None = None,
        run_pdf_ua_seed_verify: bool | None = None,
        emit_reading_order_trace: bool | None = None,
        emit_pdf_structure_trace: bool | None = None,
    ) -> "AccessibilityRunResult": ...
```

```python
# python/fullbleed/accessibility/runtime.py

def render_accessible_document(
    *,
    engine: AccessibilityEngine,
    body_html: str,
    css_text: str,
    out_dir: str,
    stem: str,
    profile: str = "cav",
    **kwargs,
) -> "AccessibilityRunResult": ...

def render_document_artifact_bundle(
    *,
    engine: AccessibilityEngine,
    artifact,  # DocumentArtifact-like, must support to_html()
    css_text: str,
    out_dir: str,
    stem: str,
    a11y_mode: str | None = "raise",
    **kwargs,
) -> "AccessibilityRunResult": ...
```

## Wrapper Policy Scaffold (Must Enforce)

`AccessibilityEngine` should enforce these invariants:

1. Constructor does not expose `pdf_profile`.
2. Internally instantiate `fullbleed.PdfEngine(..., pdf_profile="pdfua")` (or `"tagged"` if the alias path is avoided internally).
3. Report/run-report metadata must include:
   - `pdf_ua_targeted = true`
   - `engine_pdf_profile_effective = "tagged"` (if that is the actual engine enum in this sprint)
   - `engine_pdf_profile_requested = "pdfua"` (or equivalent explicit intent)
4. Strict mode behavior:
   - missing `document_lang` / `document_title` => raise before render
5. Warn mode behavior:
   - missing metadata => default + warning + run-report diagnostic

## Result/Run Report Scaffold

Define a stable bundle result shape now so examples/scaffolds can adopt it once and stop churning.

### `AccessibilityRunResult` (Python object)

Recommended fields:

- `ok: bool`
- `pdf_ua_targeted: bool`
- `paths: dict[str, str]`
- `verifier_report: dict | None`
- `pmr_report: dict | None`
- `pdf_ua_seed_report: dict | None`
- `reading_order_trace: dict | None`
- `pdf_structure_trace: dict | None`
- `run_report: dict`
- `contract_fingerprint: str | None`
- `warnings: list[str]`

### `_run_report.json` (file contract)

Add/standardize keys:

- `pdf_ua_targeted`
- `engine_pdf_profile_requested`
- `engine_pdf_profile_effective`
- `document_lang`
- `document_title`
- `html_path`
- `css_path`
- `pdf_path`
- `pdf_ua_seed_verify_path`
- `reading_order_trace_path`
- `pdf_structure_trace_path`
- `render_preview_png_paths`
- `engine_a11y_verify_path`
- `engine_pmr_path`
- `engine_a11y_verify_ok`
- `engine_pmr_ok`
- `engine_pmr_score`
- `audit_contract_fingerprint`
- `audit_registry_hash`
- `wcag20aa_registry_hash`
- `section508_html_registry_hash`

This keeps audit provenance visible in the workflow-level artifact, not only inside verifier/PMR reports.

## File/Path Conventions (Scaffold to Reuse)

Use the existing CAV/scaffold naming pattern to avoid churn:

- `${stem}.html`
- `${stem}.css`
- `${stem}.pdf`
- `${stem}_pdf_ua_seed_verify.json`
- `${stem}_reading_order_trace.json`
- `${stem}_pdf_structure_trace.json`
- `${stem}_a11y_verify_engine.json`
- `${stem}_pmr_engine.json`
- `${stem}_run_report.json`
- `${stem}_page{n}.png` (preview PNGs)

Avoid introducing a second naming convention in this sprint.

## Migration Scaffold (CAV + Accessible Template)

### Marriage CAV (`keenan_coutney_marriage_cav`)

Migration sequence (minimize risk):

1. Introduce `AccessibilityEngine` in `report.py`.
2. Route HTML/CSS/PDF/audit emission through `render_bundle(...)`.
3. Preserve:
   - pre-render `A11yContract` validation
   - component mount validation
   - claim-evidence sidecar generation
   - parity/source-analysis sidecars
4. Keep existing run-report keys for one sprint and add new keys (do not break downstream scripts immediately).

### `fullbleed new accessible` scaffold

Migration sequence:

1. Replace direct `PdfEngine` calls with `AccessibilityEngine`.
2. Keep the same output filenames.
3. Keep claim-evidence sidecar behavior and automatic verifier/PMR reports.
4. Update README wording to "PDF/UA-targeted tagged PDF".

## Test Scaffold (Sprint-Blocking)

Create/extend tests before broad migration.

### Unit tests (Python)

Recommended new file:

- `tests/test_fullbleed_accessibility_engine.py`

Cases:

1. `AccessibilityEngine` forces PDF/UA-targeted profile
   - no `pdf_profile` override accepted
2. metadata strict mode raises on missing `lang/title`
3. metadata warn mode defaults and records warning
4. `render_bundle(...)` emits expected artifact set
5. verifier/PMR default-on behavior works
6. contract provenance present in run report

### Integration tests (Python)

Recommended file:

- `tests/test_fullbleed_accessibility_bundle_integration.py`

Cases:

1. minimal semantic doc via `fullbleed.ui.accessibility` -> bundle output
2. `a11y_report` passthrough reaches engine verifier
3. claim-evidence passthrough reaches engine verifier
4. preview PNG passthrough triggers contrast seed path
5. non-visual PDF observability artifacts are emitted and referenced in run report

### Regression checks (Examples)

Use the marriage CAV as the canary:

- page parity unchanged
- natural pass unchanged (except expected readiness warning)
- PMR score unchanged (or documented if intentionally changed)
- non-visual PDF observability artifacts present and parseable for CI

## Implementation Sequence (Execution Order)

Use this order to minimize churn and test flake:

1. Breadcrumb 0 (claim language text)
2. Breadcrumb 1 (package + wrapper scaffold)
3. Breadcrumb 2 (policy enforcement + metadata behavior)
4. Breadcrumb 6 partial (unit tests for wrapper policy)
5. Breadcrumb 3 (bundle API)
6. Breadcrumb 3B (non-visual PDF observability seeds)
7. Breadcrumb 6 remaining (bundle + observability tests)
8. Breadcrumb 4 + 5 (CAV/scaffold adoption)
9. Breadcrumb 7 (docs/migration)

This gets tests around the new boundary before migrating real examples.

## Explicit "Do Not Implement Yet" (Execution Guardrails)

To keep the sprint focused:

- do not create a PDF/UA conformance score
- do not add a PDF/UA registry in this sprint
- do not rework Rust engine profiles unless the wrapper approach proves insufficient
- do not rename existing verifier/PMR schemas for this sprint
- do not break existing `PdfEngine` accessibility workflows while introducing the new surface
- do not claim "screen reader tested" from engine traces alone

## Naming / Claim Language (Important)

### Runtime/Profile Language

Use precise wording in code/docs/reports:

- `pdf_ua_targeted` (or equivalent) for engine mode/intent
- `pdf_ua_claim_status` (future)
- never imply certification/conformance unless verified and explicitly stated

### Short-Term Practical Rule

From `fullbleed.accessibility`, output is:

- `PDF/UA-targeted tagged PDF` (engine intent)

not:

- `PDF/UA conformant` (unless/when a PDF verifier and claim workflow can support it)

## Sprint Scope (Recommended)

### In Scope

1. `fullbleed.accessibility` package scaffold (Python)
2. Accessibility engine wrapper with forced PDF/UA-targeted mode
3. Metadata requirements/defaults (`lang`, `title`) and fail-fast behavior
4. Built-in engine verifier + PMR invocation and report emission
5. Non-visual PDF observability seeds (reading-order trace + PDF structure/tagging seed checks)
6. CAV/scaffold integration path using the new surface (at least one real sample)
7. Claim/report labeling updates for PDF/UA-targeted wording
8. Tests + observability gates

### Out of Scope (Backlog)

1. Deep PDF semantic verifier (StructTree/role mapping completeness)
2. PDF/UA clause-by-clause registry (future epic)
3. OCR/content-quality checks
4. Deeper WCAG hybrid confidence work (backlogged for now)
5. Full screen-reader runtime automation/emulation (viewer/OS AT integration)

## Implementation Theory

## Theory 1: Accessibility Surface Must Be Opinionated

If `fullbleed.accessibility` exposes the same knobs as `PdfEngine`, users will bypass the accessibility path by accident.

This surface should:

- force the PDF/UA-targeted profile
- expose safe defaults
- make verifier/PMR/report sidecars default-on
- make disabling accessibility checks explicit and noisy

## Theory 2: Keep the Contract Authority in Rust

Do not move policy into Python just because the new surface is Python-first.

Reuse the existing immutable audit contract crate and report provenance:

- contract fingerprint
- registry hashes
- rule IDs / verdict semantics

This preserves defensibility and reproducibility.

## Theory 3: Accessibility Workflow = Render + Audit + Evidence

For this surface, rendering is incomplete without audit artifacts.

A successful accessibility render should produce a bundle:

- HTML artifact
- CSS artifact
- PDF artifact (PDF/UA-targeted mode)
- a11y verifier report
- PMR report
- run report (paths/status/contract fingerprint)

This matches our observability standard and keeps failures explainable.

## Theory 4: Non-Visual PDF Gates Are a Screen-Reader Proxy, Not a Screen Reader

A real screen reader runs through a viewer + OS accessibility stack. FullBleed should not pretend to replace that in-engine.

What we can and should provide in this sprint:

- deterministic reading-order traces
- deterministic PDF structure/tagging seed facts
- CI-parseable seed verifier outputs

This gives us strong non-visual observability and fail-fast signals, while preserving an honest boundary:

- engine traces/supporting seeds = CI proxy
- manual AT verification = final confidence

## Proposed Sprint Plan (1 Sprint, Breadcrumbed)

## Sprint Goal

Ship a usable `fullbleed.accessibility` path that renders PDF/UA-targeted output and emits audit artifacts by default, with a clear API boundary and no claim ambiguity.

## Breadcrumb 0: Vocabulary + Contract Labeling

Goal:
- freeze terminology for "PDF/UA-targeted" vs conformance claim

Deliverables:
- `docs/specs/pdf_ua_claim_language.md` (or fold into existing claim-language doc with a new section)
- report/tooling field names reviewed for wording consistency

Failure gates:
- no report field/docs string implies PDF/UA conformance without claim workflow

## Breadcrumb 1: `fullbleed.accessibility` Python Surface

Goal:
- introduce the canonical accessibility runtime package

Deliverables:
- `python/fullbleed/accessibility/__init__.py`
- `python/fullbleed/accessibility/engine.py`
- `python/fullbleed/accessibility/runtime.py` (optional helper/result models)

Stories:
- `AccessibilityEngine` wrapper around `fullbleed.PdfEngine`
- preserve access to metadata properties (`document_lang`, `document_title`)
- expose `document_metadata()` passthrough

Failure gates:
- wrapper always initializes engine in PDF/UA-targeted mode
- attempts to override to non-accessible profile from this surface fail

## Breadcrumb 2: Forced PDF/UA-Targeted Engine Policy

Goal:
- make the accessibility surface unambiguous at runtime

Deliverables:
- explicit wrapper policy:
  - forced profile (`pdfua` alias or explicit new enum if introduced)
  - metadata requirements/defaults
  - audit-default behavior

Stories:
- if `lang`/`title` absent, choose:
  - default + warning, or
  - fail-fast in strict mode
- add `strict` / `warn` mode for accessibility runtime wrapper

Failure gates:
- emitted PDF from `AccessibilityEngine` is never untagged
- reports include profile intent (`pdf_ua_targeted=true`)

## Breadcrumb 3: Accessibility Render Bundle API

Goal:
- make audit artifacts default, not optional afterthoughts

Deliverables:
- `AccessibilityEngine.render_bundle(...)` (name flexible)
- emits:
  - `.html`
  - `.css`
  - `.pdf`
  - (optionally/default) non-visual PDF observability artifacts (see Breadcrumb 3B)
  - `_a11y_verify.json`
  - `_pmr.json`
  - `_run_report.json`

Stories:
- support render preview PNG path pass-through so contrast seed runs automatically
- accept claim-evidence and `a11y_report` (pre-render diagnostics) passthrough

Failure gates:
- run report records all artifact paths + status
- verifier/PMR run by default unless explicitly disabled

## Breadcrumb 3B: Non-Visual PDF Observability Seeds

Goal:
- add deterministic, non-visual PDF accessibility observability suitable for CI and triage (without claiming screen-reader equivalence)

Deliverables:
- PDF structure/tagging seed verifier artifact (JSON)
- reading-order trace artifact (JSON)
- PDF structure trace artifact (JSON)
- run-report integration for all three artifact paths/status

Stories:
- expose a debug/introspection path in engine or wrapper sufficient to serialize:
  - logical reading order trace (sequence of tagged content items/containers)
  - coarse PDF structure/tagging facts (tagged present, root/lang/title metadata where available, role/tree seeds)
- produce a `pdf_ua_seed_verify` report with explicit `seed-only` wording and confidence labels
- ensure artifacts are emitted by `render_bundle(...)` by default (or explicit accessibility-default-on flag)

Failure gates:
- artifacts are emitted and parseable JSON
- run report references all non-visual PDF observability artifact paths
- no claim language implies AT/runtime screen-reader validation
- marriage CAV emits traces deterministically enough for CI presence/shape checks

## Breadcrumb 4: `fullbleed.ui.accessibility` Integration Path

Goal:
- ensure the new runtime surface is ergonomic with existing semantic authoring APIs

Deliverables:
- helper accepting `DocumentArtifact` / `to_html()` output
- examples showing `fullbleed.ui.accessibility` + `fullbleed.accessibility.AccessibilityEngine`

Stories:
- zero-friction path for current CAV harnesses
- preserve `a11y_mode` validation before render

Failure gates:
- marriage CAV can render through the new surface with same pass results

## Breadcrumb 5: CAV + Scaffold Adoption (Canary)

Goal:
- prove the new surface on real workflows without breaking the semantics stack

Deliverables:
- migrate `examples/accessibility_test/keenan_coutney_marriage_cav/report.py`
  - or add a side-by-side path behind a flag for one sprint
- update `fullbleed new accessible` scaffold to use `fullbleed.accessibility`

Failure gates:
- CAV remains page-parity stable (`1 page`)
- verifier gate `ok=true`
- PMR gate `ok=true`
- natural pass remains clean except allowed readiness warning(s)

## Breadcrumb 6: Tests + Observability

Goal:
- lock the new boundary with explicit failure modes

Deliverables:
- Python tests for wrapper policy:
  - forced profile
  - metadata handling
  - report bundle emission
  - verifier/PMR default-on
  - non-visual PDF observability artifact emission/path recording
- integration test with a minimal semantic doc

Failure gates:
- `pytest` passes
- bundle artifact paths stable
- contract fingerprint emitted in run report
- non-visual PDF observability artifacts emitted when enabled/defaulted for accessibility bundle

## Breadcrumb 7: Docs + Migration Notes

Goal:
- make the new accessibility stack obvious and reduce user confusion

Deliverables:
- `docs/ui-accessibility.md` update (authoring + runtime split)
- `docs/python-api.md` update for `fullbleed.accessibility`
- `docs/cli.md` note if/when CLI wrapper is added
- migration note:
  - `fullbleed.ui.accessibility` = authoring
  - `fullbleed.accessibility` = accessibility runtime/output stack

Failure gates:
- docs examples compile/run (or are smoke-checked)

## Sprint Acceptance Criteria (Definition of Done)

- `fullbleed.accessibility` exists and is the canonical accessibility runtime surface
- its engine path always emits PDF/UA-targeted tagged PDFs (no non-accessible profile from this API)
- audit artifacts (verifier + PMR) are emitted by default with contract provenance
- non-visual PDF observability artifacts (reading-order trace + PDF structure/tagging seed outputs) are emitted by default or accessibility-default-on policy and recorded in run reports
- at least one real CAV sample runs through the new surface unchanged in semantics and page parity
- wording is explicit about `PDF/UA-targeted` vs conformance claim
- tests and docs cover the new boundary

## Open Questions (Resolve at Sprint Kickoff)

1. Should we introduce a distinct engine enum/profile for `PdfUaTargeted` vs current `Tagged`, or keep engine internals as `Tagged` and only clarify at the Python accessibility surface?
2. Should `AccessibilityEngine` fail hard when `document_lang` / `document_title` are missing, or default + warn in non-strict mode?
3. Do we want `fullbleed.accessibility` to expose a single high-level `render_bundle(...)` only, or also low-level passthroughs (`emit_html`, `emit_css`, `render_pdf`)?
4. Should the first sprint include a CLI alias (for example `fullbleed accessibility render`) or stay Python-only?

## Recommended Decisions (SME Default)

1. Keep engine internals as `Tagged` for this sprint; make `fullbleed.accessibility` the semantic boundary and use `PDF/UA-targeted` wording everywhere.
2. Default + warn for missing metadata in non-strict mode; fail in strict mode.
3. Ship both:
   - `render_bundle(...)` (canonical)
   - minimal low-level passthroughs for advanced callers
4. Keep this sprint Python-first; backlog CLI alias unless migration pressure appears immediately.

## Backlog (Post-Sprint)

- PDF artifact verifier seeds (StructTree presence, doc language/title in PDF catalog, mark info/tagging checks)
- PDF/UA registry/coverage model (analogous to WCAG + Section 508 scoped registries)
- CLI accessibility workflow wrapper
- deeper PDF semantics parity checks (HTML semantic intent -> PDF tags)
- golden enrollment for PDF/UA-targeted canaries
