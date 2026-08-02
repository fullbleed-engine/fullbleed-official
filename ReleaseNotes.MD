# Fullbleed Release Notes - 2.0.0

Status: release candidate

Target date: August 1, 2026

## Summary

Fullbleed 2.0 removes every third-party software dependency from the shipped
Rust, Python runtime, and Python build graphs while preserving the documented
rendering, inspection, composition, accessibility, CLI, and packaging
capabilities. The root Rust crate now depends only on the Fullbleed-authored
`fullbleed_audit_contract` crate; that crate has no dependencies of its own.
Optional features add no packages.

Redistributed fonts, CSS, and icon artwork remain third-party assets under
their existing licenses. They are not linked software dependencies and are
still covered by `THIRD_PARTY_LICENSES.md` and the font-license audit.

## Native Replacements

- Replaced external hashing, Base64, fixed-point, math, compression, ordered
  parallelism, JSON, XML, HTML, image, JPEG, PDF, SVG, CSS, font, shaping, and
  raster libraries with bounded in-repo implementations.
- Replaced the external Python binding generator with a CPython 3.10+ Stable
  ABI boundary and a standard-library-only compatibility facade.
- Replaced the external Python build backend with the in-tree
  `fullbleed_build_backend`, including deterministic wheel/sdist creation,
  editable installs, `RECORD`, license files, runtime resources, and CycloneDX
  SBOM generation.
- Retained `python` and `svg_raster` as compatibility feature names; neither
  feature adds a Cargo package.

The detailed replacement/evidence matrix is in
`docs/2.0-dependency-removal.md`.

## Compatibility

- Preserves the public `PdfEngine`, `AssetKind`, `Asset`, `AssetBundle`, and
  `WatermarkSpec` classes, free functions, explicit signatures, properties,
  batch methods, and `fullbleed._fullbleed` import surface.
- Preserves `cp310-abi3` packaging for standard CPython 3.10 through 3.14 and
  the 15-platform wheel matrix.
- Preserves the Rust 1.85 minimum toolchain, deterministic output contracts,
  PDF profile behavior, SVG raster fallback, native image emission, and the
  validated CSS fixture surface.
- Keeps the `fullbleed[api]` compatibility extra valid with no required
  packages; the ignored Escambia HTTP prototype was never shipped.

## Package Versions

- `fullbleed 2.0.0` on crates.io and PyPI.
- `fullbleed_audit_contract 0.1.2` on crates.io. This dependency-free patch
  must be published before the main crate.

## Verification Baseline

The release candidate has a 658-test all-feature Rust suite and a 248-test
Python suite. The Rust suite reports 657 passed and one explicit local-font
opt-in ignored. It also carries the complete 376-test style module and all 86
full CSS parity fixtures. Local release validation passes the Rust 1.85
packaged consumer, both Cargo publication dry runs, a clean isolated wheel
install/render, and byte-identical checkout/sdist wheels. Final release
validation uses:

```powershell
cargo fmt --all --check
cargo test --locked
cargo test --locked --features python,svg_raster
cargo check --locked --all-targets --all-features
cargo tree -e normal --all-features
python -m pytest -q
python tools\generate_css_parity_status.py --check --json
python tools\check_license_integrity.py --json
python tools\check_release_metadata.py --expected-version 2.0.0 --json
rustup run 1.85.0 python tools\smoke_crates_consumer.py --expected-version 2.0.0 --json
cargo publish --locked --dry-run --manifest-path crates\fullbleed_audit_contract\Cargo.toml
# Before audit-contract 0.1.2 reaches crates.io, supply its local path through
# Cargo's patch.crates-io configuration; after propagation, run this normally.
cargo publish --locked --dry-run
python build_backend\fullbleed_build_backend.py wheel --out dist
python build_backend\fullbleed_build_backend.py sdist --out dist
```

The exact `v2.0.0` commit must also pass the installed-wheel matrix, package
metadata/tag checks, golden renders, PDF profile validation, and clean-worktree
gate before publication.

## Publication

Publication is intentionally pending. The release workflows publish the audit
contract first, wait for registry propagation, then publish the main Rust crate,
15 ABI3 wheels, one source distribution, and matching GitHub release assets.
