# Backlog

Release cut for `0.6.5` is based on the current accessibility/PDF/UA-targeted stack. The items below are intentionally deferred.

## Accessibility / Audit Depth

- Deepen `WCAG 2.0` hybrid seeds into stronger deterministic/runtime checks for highest-impact criteria (`1.4.3`, `1.3.2`, `2.1.1`).
- Add richer claim-evidence ingestion ergonomics (CLI + sidecar schema examples) beyond current Python/harness patterns.
- Add verifier/PMR CLI surfaces (`fullbleed a11y verify`, `fullbleed pmr verify`) on top of engine-native APIs.

## PDF/UA-Targeted Output Stack

- Move non-visual PDF trace collection deeper into engine render-time instrumentation and treat it as the primary CI trace path.
- Deepen PDF seed verification (`MCID` integrity, StructTree parent/child consistency, tag-role coverage summaries).
- Add a PDF/UA-targeted registry/coverage model (analogous to WCAG / Section 508 scoped registries).
- Strengthen render-time vs post-render PDF trace cross-checks.

## Accessibility Example Corpus / Goldens

- Add CI goldens for the 3-track `_accessibility_examples` corpus (`cav_golden`, `known_failure_legacy`, `wild_best_attempt_html`).
- Gate on non-visual trace presence/schema plus selected count/ratio thresholds.
- Add more “wild best attempt” samples (tables/forms/signatures) for PDF-output-centric analysis.

## Runtime / Packaging

- Decide whether engine-native HTML artifact emission should inject `<link rel=\"stylesheet\">` (currently handled in the accessibility wrapper).
- Clarify/align UI-layer vs engine-layer artifact emission ergonomics (`DocumentArtifact.emit_*` vs engine emitters).
- Publish contract fingerprint pinning workflow in release/CI docs.

## Docs / Release Hardening

- Expand `fullbleed.accessibility` docs (trace schemas, PDF seed checks, claim evidence patterns).
- Add a release checklist doc for accessibility/PDF/UA-targeted regression + PAC/manual validation steps.
- Enroll more examples/canaries in golden regression coverage.
