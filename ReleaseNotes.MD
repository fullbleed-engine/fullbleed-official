# Fullbleed Release Notes - 1.6.2

Status: release candidate

Release date: pending

## Summary

Fullbleed 1.6.2 is a compatibility and packaging patch for Cargo consumers. It
prevents a fresh consumer lockfile from selecting mutually incompatible alpha
releases in the Lightning CSS dependency family and preserves the declared
Rust 1.85 floor. Rendering behavior, public APIs, the MIT license, and the
supported Python/platform matrix are unchanged from 1.6.1.

## Cargo Packaging

- Exact-pinned `lightningcss` to the tested `1.0.0-alpha.70` release.
- Added an exact compatibility constraint for `parcel_selectors 0.28.2`.
- Exact-pinned `time 0.3.45`, the newest tested release in that line whose
  declared MSRV remains below Fullbleed's Rust 1.85 floor.
- Replaced six post-1.85 `let` chains with behavior-equivalent nested
  conditionals.
- Added a release gate that packages Fullbleed, extracts the exact `.crate`,
  consumes it from a temporary project with no repository lockfile, and runs
  `cargo check` using Rust 1.85.
- Verifies that the fresh consumer resolves `fullbleed 1.6.2`,
  `fullbleed_audit_contract 0.1.1`, and the intended CSS and time dependencies.
- Keeps the already-published audit-contract crate at version 0.1.1.

## Python Packaging

- Keeps `abi3-py310` and CPython 3.10 through 3.14 support.
- Keeps the existing 15-wheel platform matrix and source distribution.
- Aligns the Python distribution version with the patched Rust crate at 1.6.2.

## Licensing

Both Fullbleed-authored crates and the Python distribution remain MIT
licensed. Third-party dependencies retain their own licenses.

## Release Gates

The exact `v1.6.2` release commit must pass:

```powershell
cargo fmt --all --check
cargo test --locked
cargo test --locked --features python,svg_raster
python -m pytest -q
python tools/check_license_integrity.py --json
python tools/check_release_metadata.py --expected-version 1.6.2 --json
rustup toolchain install 1.85.0 --profile minimal
rustup run 1.85.0 python tools/smoke_crates_consumer.py --expected-version 1.6.2 --json
cargo publish --locked --dry-run
python -m maturin build --release --locked
python -m maturin sdist
python -m twine check target/wheels/*
```

Registry publication remains an explicit manual action after the complete
artifact and installed-package gates pass.
