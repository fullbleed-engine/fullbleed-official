# Fullbleed Release Notes - 0.6.5

Release date: February 28, 2026

## Summary

`0.6.5` is a stabilization and release-hardening cut over the `0.6.x` accessibility stack.

This release focuses on deterministic document-level artifact emission, stronger scaffold defaults, and preflight metadata alignment for packaging and docs.

## Key Changes

### Document-Level CSS Metadata + Emission

- Added and hardened document-level CSS metadata across UI and engine surfaces:
  - `document_css_href`
  - `document_css_source_path`
  - `document_css_media`
  - `document_css_required`
- Artifact emitters now support metadata-driven linking and fail-fast behavior when CSS metadata is required but missing.
- Emission observability now includes CSS link outcomes (`injected`, `preexisting`, `href`, `media`) in run reports.

### Accessibility Runtime Alignment

- `fullbleed.accessibility.AccessibilityEngine` now carries CSS metadata through artifact emission and run-report surfaces.
- HTML artifact emission preserves doc-level semantics while ensuring linked-CSS output is explicit and machine-verifiable.

### Scaffold Upgrades

- `fullbleed init` scaffold `report.py` now emits:
  - `output/report.html`
  - `output/report.css`
  - `output/report.pdf`
  with engine-level CSS metadata defaults.
- `fullbleed new accessible` scaffold now defaults to document-level CSS metadata and reports CSS-link observability in run reports.
- `fullbleed new invoice` and `fullbleed new statement` now include runnable `report.py` entrypoints with metadata-driven HTML/CSS/PDF emission and run reports.

### Testing + Preflight Coverage

- Added scaffold coverage tests for metadata-aware local templates.
- Extended existing scaffold tests to assert CSS metadata defaults in generated sources.

### Release/Repo Hygiene

- Updated package metadata and docs version references from `0.6.0` to `0.6.5`.
- Added repo-wide ignore for generated Python artifact pattern `*.pyc.*`.

## Preflight Checklist (This Pass)

- [x] Version metadata bumped (`Cargo.toml`, `pyproject.toml`)
- [x] Docs/version references updated
- [x] Release notes refreshed for `0.6.5`
- [x] Scaffold template tests passing
- [x] `new invoice` / `new statement` smoke scaffolds generated and rendered

## Build Artifact Notes

Built in this preflight pass:

- `dist/wheels-win/fullbleed-0.6.5-cp311-cp311-win_amd64.whl`
  - size: `8,000,005` bytes
  - sha256: `3630035DEE8CDA5A6B16E9AE34B06D1A97710CD54312E634F2BD04B9B9850E73`
- `dist/wheels-wsl2/fullbleed-0.6.5-cp310-cp310-manylinux_2_35_x86_64.whl`
  - size: `8,370,548` bytes
  - sha256: `57F9C8D3D946F8A4B2F792998E8061239BD877BE9AD3D3B0DFC7C639B869FA6A`
- `dist/wheels-wsl2/fullbleed-0.6.5-cp311-cp311-manylinux_2_35_x86_64.whl`
  - size: `8,371,869` bytes
  - sha256: `58F10CA13DE77D48AD38F3E9320EC166946A7F27FE90D21EC3430C03BBAA6560`

## Upgrade Guidance

1. Prefer metadata-aware artifact emission (`emit_artifacts`) for all scaffolded report entrypoints.
2. Keep `document_css_required=True` in CI-sensitive pipelines to fail fast on packaging drift.
3. For accessibility workflows, continue treating output as PDF/UA-targeted and gate with verifier + PMR + seed traces plus manual review where required.
