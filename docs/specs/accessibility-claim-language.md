# Accessibility Claim Language Policy (Draft)

## Purpose

Standardize wording across FullBleed CLI, Python APIs, reports, docs, and examples so we do not over-claim accessibility conformance.

This policy applies to:

- accessibility verifier reports (`fullbleed.a11y.verify.v1`)
- Paged Media Ranker (PMR) reports (`fullbleed.pmr.v1`)
- CLI output and CI summaries
- example run reports and parity reports

## Core Rule

`Paged Media Rank` and `Accessibility Verifier` results are not legal conformance certification.

They are machine-verifiable engineering signals and evidence outputs.

## Approved Terms (Use These)

- `machine-verified`
- `machine-verifiable subset`
- `manual review required`
- `conformance-subset status`
- `compatibility rank`
- `paged media rank`
- `gate result`
- `evidence report`
- `coverage report`
- `manual debt`
- `supplemental adapter evidence`

## Restricted / Avoided Terms (Do Not Use Casually)

Do not use these terms unless a report is explicitly scoped and supported for that exact claim:

- `508 compliant`
- `WCAG compliant`
- `certified accessible`
- `fully conformant`
- `passes Section 508` (unless tied to explicit criterion-level process and human review)
- `guaranteed accessible`

## Required Disclaimers

## For PMR Reports

PMR outputs must include wording equivalent to:

- "Paged Media Rank is an operational compatibility score, not a legal conformance determination."

## For Accessibility Verifier Reports

Verifier outputs must include wording equivalent to:

- "This report evaluates a machine-verifiable subset and may require manual review for full conformance assessment."

## For Browser Adapter Results (Lighthouse/axe)

Adapter outputs must include wording equivalent to:

- "Browser adapter audits are supplemental evidence and do not replace engine-native checks or manual review."

## Standard Field Names (Preferred)

Use these field names in schemas and APIs:

- `conformance_status`
- `compatibility_rank` or `paged_media_rank`
- `manual_review_debt`
- `coverage`
- `gate`

Avoid ambiguous fields like:

- `compliance_score`
- `accessibility_certainty`

## Example Wording

Good:

- "Gate failed: broken ARIA references detected (`fb.a11y.aria.reference_target_exists`)."
- "PMR score 88 (good), confidence 84. Manual review debt remains."
- "Conformance status: `manual_review_required` (machine subset passed)."

Bad:

- "This document is Section 508 compliant."
- "Accessibility score 95 means conformant."
- "Certified accessible by FullBleed."

## Exceptions

If FullBleed later supports a formal conformance workflow with explicit human sign-off and scope controls, that workflow must define:

- the exact claim language
- required evidence and review steps
- responsibility/ownership boundaries

Until then, this policy remains strict.

