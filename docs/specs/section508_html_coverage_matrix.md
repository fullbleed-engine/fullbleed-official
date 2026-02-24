# Section 508 HTML Coverage Matrix (Scoped E205 Profile)

This document summarizes current FullBleed coverage against the scoped Section 508 registry in `docs/specs/section508_html_registry.v1.yaml`.

It is a planning artifact for implementation sequencing, not a Section 508 conformance claim.

## Scope (Important)

- Registry scope: `section508.revised.e205.html`
- Focus: FullBleed HTML deliverables via the Revised Section 508 electronic content path (`E205`) with `WCAG 2.0 A/AA + conformance requirements` incorporated by reference (`E205.4`)
- This is **not** a complete registry of all Revised 508 chapters/provisions across all ICT domains (hardware/software/support docs/etc.)

## Basis

- Source of truth:
  - `docs/specs/section508_html_registry.v1.yaml`
  - `docs/specs/wcag20aa_registry.v1.yaml` (inherited by reference)
- Section 508 profile structure:
  - `6` Section 508-specific scoping/claim entries
  - `43` inherited WCAG 2.0 AA entries (A/AA success criteria + conformance requirements)
  - `49` total coverage entries

## Summary (Current Mapping Coverage)

- `49` total scoped Section 508 profile entries
- `49` entries have implemented mappings (`100.0%`)
- `0` entries are unmapped (`0.0%`)

Breakdown:

- Section 508-specific entries: `6 / 6` implemented (`100.0%`)
- Inherited WCAG entries: `43 / 43` implemented (`100.0%`)

## Matrix: By Segment x Implementation State

| Segment | Implemented | Unmapped | Total |
|---|---:|---:|---:|
| Section 508-specific (E205 profile) | 6 | 0 | 6 |
| Inherited WCAG 2.0 AA (via E205.4) | 43 | 0 | 43 |
| Total | 49 | 0 | 49 |

## Implemented-Mapped Entries (Current)

Section 508-specific:

- `s508.e205.2.public_facing_content_applicability` - partial via `fb.a11y.claim.section508.public_facing_content_applicability_seed`
- `s508.e205.3.agency_official_communications_applicability` - partial via `fb.a11y.claim.section508.official_communications_applicability_seed`
- `s508.e205.3.nara_exception_applicability` - partial via `fb.a11y.claim.section508.nara_exception_applicability_seed`
- `s508.e205.4.wcag20aa_incorporation` - partial/supporting via `fb.a11y.claim.wcag20aa_level_readiness`
- `s508.e205.4.non_web_document_exception_applicability` - partial (HTML-path `not_applicable` seed) via `fb.a11y.claim.section508.non_web_document_exceptions_html_seed`
- `s508.e205.4.1.word_substitution_for_non_web_documents` - partial (HTML-path `not_applicable` seed) via `fb.a11y.claim.section508.non_web_document_exceptions_html_seed`

Inherited WCAG coverage:

- See `docs/specs/wcag20aa_coverage_matrix.md` (`Currently Implemented-Mapped WCAG Entries`)

## Current Gaps (Highest Impact)

1. Section 508-specific entries are now seeded/mapped, but remain manual-evidence-heavy (especially `E205.2`, `E205.3`, and `E205.3 Exception`)
2. Non-web document exception and word-substitution entries are only HTML-path `not_applicable` seeds today (future PDF/non-web path still needs dedicated logic)
3. Inherited WCAG mapping gap is now closed for this scoped profile; remaining risk is confidence depth (hybrid/manual-evidence-heavy coverage), not mapping completeness

## Observability Status

- Engine verifier and prototype verifier now emit `coverage.section508` using the embedded audit contract registry (`section508_html_registry.v1`)
- This coverage block is scoped and compositional:
  - Section 508-specific entries
  - inherited WCAG coverage by reference
