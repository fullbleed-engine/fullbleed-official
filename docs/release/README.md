# Fullbleed Release Documentation

This directory is the release-operations layer for Fullbleed. It is separate
from feature documentation so release decisions can be audited without reading
engine internals.

## 2.5.6 Documents

- `docs/release/2.5.6-runbook.md`: single-copy synthesized-bold text, affine glyph previews, and retained extraction checks.
- `tools/smoke_text_extraction.py`: installed-wheel styled PDFs, independent text extraction, and native preview coverage checks.

## 2.5.5 Documents

- `docs/release/2.5.5-runbook.md`: finalized gradient previews, source PDF preservation, and independent visual checks.
- `tools/smoke_gradient_preview.py`: installed-wheel color probes with retained PDFs and deterministic PNGs.

## 2.5.4 Documents

- `docs/release/2.5.4-runbook.md`: native Standard 14 preview spacing and retained visual checks.
- `tools/smoke_base14_preview.py`: actual PDF/PNG comparisons against explicit reference widths, with negative controls.

## 2.5.3 Documents

- `docs/release/2.5.3-runbook.md`: reproducibility baseline validation, failure controls, and installed-package verification.
- `tools/smoke_repro_check.py`: retained real-process checks for passing, changed, malformed, missing, and conflicting records.

## 2.5.2 Documents

- `docs/release/2.5.2-runbook.md`: MCP input guidance, side-effect hints, and public package verification.
- `docs/agent-discovery.md`: Agent Skill installation and MCP discovery procedures.
- `ReleaseNotes.MD`: current release summary and retained earlier notes.

## 2.5.1 Documents

- `docs/release/2.5.1-runbook.md`: strict MCP client compatibility and installed-wheel interoperability checks.
- `tools/mcp-client-smoke/README.md`: official client SDK reproduction and validation.

## 2.5.0 Documents

- `docs/release/2.5.0-runbook.md`: watch-mode release, package verification, and publication procedure.
- `docs/render-watch.md`: automatic rebuilds, explicit dependencies, result streaming, and limits.
- `ReleaseNotes.MD`: retained 2.5.0 release summary.

## 2.4.0 Documents

- `docs/release/2.4.0-runbook.md`: candidate release and print-profile verification gate.
- `docs/release/2.4.0-validation-report.md`: candidate verification results and retained evidence.
- `docs/pdf-vt.md`: print identity, record composition, DPM, and reuse-hint contracts.
- The `v2.4.0` GitHub release retains the 2.4.0 notes and verification boundaries.

## 2.3.1 Documents

- `docs/release/2.3.1-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub release procedure.
- `ReleaseNotes.MD`: historical 2.3.1 notes are retained in the `v2.3.1` GitHub release.

## 2.3.0 Documents

- `docs/release/2.3.0-runbook.md`: historical Cargo, PyPI, GitHub, Agent Skill, MCP package, and MCP Registry release procedure.
- `docs/agent-discovery.md`: discovery targets, publication ordering, and claim boundaries.

## 2.2.5 Documents

- `docs/release/2.2.5-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub
  release procedure.
- `docs/performance-pass-2026-08-04.md`: compiled content-reflow architecture, exact-parity
  evidence, and the independently measured 8,095.8-pages/s hot median (10,355.3-pages/s best).
- `docs/performance-architecture.md`: guarded flow programs, native PDF page shaders, and the
  remaining partial-reflow/vector-IR boundary.
- The `v2.2.5` GitHub release preserves its historical release notes.

## 2.2.4 Documents

- `docs/release/2.2.4-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub
  release procedure. `v2.2.5` supersedes it with fragmentation, named-string, compression API,
  and Python discovery hardening.

## 2.2.3 Documents

- `docs/release/2.2.3-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub
  release procedure. `v2.2.4` supersedes it with compiled content reflow.
- `docs/css-coverage.md`: current 1,642-pass, zero-failure IronPress parity evidence and
  remaining reference disputes.
- `docs/performance-architecture.md`: compiled variable-data coordinate-state support and the
  remaining form-XObject/vector-compiler boundary.

## 2.2.2 Documents

- `docs/release/2.2.2-runbook.md`: historical parity, compiler, Cargo, PyPI, wheel-matrix, and
  GitHub release procedure. `v2.2.3` supersedes it for component-validation reliability.

## 2.2.1 Documents

- `docs/release/2.2.1-runbook.md`: historical, unpublished candidate procedure. The public tag
  failed closed in debug test validation before any registry publication and was superseded by
  `v2.2.2`.

## 2.2.0 Documents

- `docs/release/2.2.0-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub
  release procedure.
- `docs/performance-pass-2026-08-04.md`: measured font-subsetting, compiled-link,
  fixed-copy virtualization, and distinct fixed-geometry variable-data results.
- `docs/performance-architecture.md`: current performance scope and the vector compiler,
  typed binding, linker, and shader roadmap.
- The `v2.2.0` GitHub release preserves its historical release notes.

## 2.1.0 Documents

- `docs/release/2.1.0-runbook.md`: historical Cargo, PyPI, wheel-matrix, and GitHub
  release procedure.
- `docs/performance-pass-2026-08-04.md`: measured font-subsetting, compiled-link,
  fixed-copy virtualization, and post-release fixed-geometry variable-data results.
- `docs/performance-architecture.md`: performance scope and the vector compiler,
  typed binding, linker, and shader roadmap.
- The `v2.1.0` GitHub release preserves its historical release notes.

## 2.0.0 Documents

- `docs/release/2.0.0-runbook.md`: historical MIT, Cargo, PyPI, wheel-build, and
  registry publication procedure.
- `docs/2.0-dependency-removal.md`: dependency-removal implementation and
  evidence ledger.

## 1.6.2 Documents

- `docs/release/1.6.2-runbook.md`: Cargo-consumer compatibility patch release
  procedure.

## 1.6.1 Documents

- `docs/release/1.6.1-runbook.md`: original MIT launch and full wheel-matrix
  release procedure.

## Historical 1.0.0 Documents

- `docs/release/1.0.0-readiness.md`: release-readiness ledger, blocker status,
  evidence, and remaining publish handoff work.
- `docs/release/1.0.0-runbook.md`: exact release sequence for final validation,
  wheel smoke, crates.io/PyPI publishing, and post-release checks.
- `docs/release/1.0.0-validation-report.md`: current standards and package
  validation evidence for the 1.0.0 handoff.
- `docs/release/claim-contract.md`: public claim language, supported scope, and
  boundaries for CSS, SVG, images, accessibility, and PDF profiles.

## Release Principles

- Release from a clean worktree with no tracked generated artifacts.
- Build and smoke installed artifacts, not only editable/local extensions.
- Keep generated output out of the release commit unless it is explicitly
  intended as a source-controlled fixture or ledger.
- Treat standards claims as evidence-backed. If a validator is not configured,
  the public claim must say so.
- Keep launch copy scoped to deterministic static PDF generation, not browser
  equivalence.

## Required Machine Gates

The final release commit must pass:

```powershell
cargo fmt --check
cargo test --locked
cargo test --locked --features svg_raster
python -m pip install --no-build-isolation --no-deps --editable .
python -m pytest -q
python tools\generate_css_parity_status.py --check --json
python tools\run_css_fixture_suite.py --fixtures inline_svg_image_mixed_run_torture --jobs 1 --json
python tools\validate_pdf_profiles.py --out output\conformance_validation_harness --download-verapdf --install-pdf-oxide --strict-external
python -m fullbleed doctor --strict --json
python -m fullbleed compliance --strict --json
python tools\check_license_integrity.py --json
python tools\check_release_worktree.py --json
cargo publish --dry-run
```

Wheel builds must use `python,svg_raster` and must be installed before smoke
tests.

The worktree gate checks both conditions required for release source hygiene:
`git status --short` must be empty, and tracked artifact violations such as
`.pytest_cache/`, `*.pyd`, `*.so`, `*.dll`, `*.dylib`, root
`fullbleed_css_fixture_*.jsonl` traces, or `fullbleed_preflight*` logs must be
absent.
