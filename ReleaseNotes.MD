# Fullbleed Release Notes - 1.6.1

Status: released

Release date: July 28, 2026

## Summary

Fullbleed 1.6.1 completes the move to the MIT License and replaces the partial
1.6.0 PyPI artifact set with full platform coverage. The engine behavior and
public rendering contract remain focused on deterministic static
HTML/CSS-to-PDF generation.

## Licensing

- Replaced the AGPL/commercial dual-license model with the MIT License.
- Updated Cargo, PyPI, runtime compliance, documentation, examples, and source
  SPDX headers to declare `MIT`.
- Removed the obsolete commercial-attestation helpers, CLI flags, and
  one-time runtime license warning.
- Retained third-party asset and dependency notices; those components remain
  under their respective licenses.

## Python Packaging

- Raised the Python floor to 3.10, matching syntax already used by the package.
- Declared and continuously tests CPython 3.10, 3.11, 3.12, 3.13, and 3.14.
- Updated PyO3 and enabled `abi3-py310`, producing one compatible wheel per
  platform for the supported CPython range.
- Added 15 release wheels: manylinux 2014 for x86-64, x86, ARM64, ARMv7,
  s390x, and ppc64le; musllinux 1.2 for x86-64, x86, ARM64, and ARMv7;
  Windows x86-64, x86, and ARM64; and macOS Intel and Apple silicon.
- Added installed-wheel smoke gates for every target architecture. Linux
  cross-built wheels execute under QEMU, while the x86-64 manylinux wheel is
  also exercised on every supported CPython release.
- Retained a source distribution so users on additional Rust-supported
  platforms can build locally when no prebuilt wheel matches.
- Superseded the initial 1.6.0 PyPI upload, which contained six platform
  wheels, with the complete 15-wheel release matrix in 1.6.1.
- Prepared PyPI trusted publishing through the protected `pypi` GitHub
  environment.

## Cargo Packaging

- Set the main crate and Python distribution version to `1.6.1` and license
  metadata to `MIT`.
- Bumped `fullbleed_audit_contract` to `0.1.1` so its newly MIT-licensed
  package can be published before the main crate.
- Added package metadata and release gates for both crates.
- Prepared crates.io publishing through the protected `crates-io` GitHub
  environment and the `CARGO_REGISTRY_TOKEN` secret.

## Release Gates

The exact `v1.6.1` release commit must pass:

```powershell
cargo fmt --all --check
cargo test --locked
cargo test --locked --features python,svg_raster
python -m pytest -q
python tools/check_license_integrity.py --json
python tools/check_release_metadata.py --expected-version 1.6.1
python -m maturin build --release --locked
python -m maturin sdist
python -m twine check target/wheels/*
```

The GitHub release workflow performs platform wheel builds, supported-version
smoke tests, and package validation from `v*` tags. Registry publication is an
explicit manual action after the complete artifact set passes.
