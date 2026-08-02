# Fullbleed Release Documentation

This directory is the release-operations layer for Fullbleed. It is separate
from feature documentation so release decisions can be audited without reading
engine internals.

## 2.0.0 Documents

- `docs/release/2.0.0-runbook.md`: current MIT, Cargo, PyPI, wheel-build, and
  registry publication procedure.
- `docs/2.0-dependency-removal.md`: dependency-removal implementation and
  evidence ledger.
- `ReleaseNotes.MD`: current 2.0.0 release summary and gate list.

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
