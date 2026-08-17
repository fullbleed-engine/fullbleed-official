# Fullbleed Release Notes - 2.3.1

Status: release

Release date: August 17, 2026

## Summary

Fullbleed 2.3.1 fixes two custom-font failures in finalized PDF previews and compiled fixed-geometry variable-data output. It preserves the dependency-free engine, deterministic fixed-point layout, existing public APIs, Base-14 binding fast path, and the 2.3.0 agent-discovery surface.

## Font and preview fixes

- Finalized-PDF previews now recover Fullbleed-generated Type 3 glyph IDs from their `gXXXX` encoding names and reconnect those synthetic-bold resources to the sibling embedded source font.
- Synthetic-bold preview lowering preserves the generated stroke strength instead of decoding Type 3 byte codes through StandardEncoding.
- The fix is internal to Fullbleed's native PDF raster path and adds no browser, system PDF stack, or system-font dependency.

## Compiled fixed-layout bindings

- Fixed-layout VDP now binds registered Identity-H fonts instead of silently leaving `{{slot}}` placeholders encoded as the original glyph IDs.
- Base-14/WinAnsi text retains the measured byte-patch linker. Registered fonts use a command-level overlay that shapes each bound value and extends the embedded subset without rerunning HTML/CSS layout or pagination.
- Binding-bearing synthetic-bold registered-font runs stay substitutable until record values are known, while immutable page paint remains shared.
- Bound values still do not reflow; templates must reserve sufficient geometry. Use compiled reflow bindings when values can wrap, resize content, or repaginate.

## Compatibility and packages

- No Rust, Python, CLI, Stable ABI, agent-contract, MCP, template, batch, fixed-copy, or reflow-binding interface is removed.
- Core `fullbleed` retains no third-party Python runtime dependency and adds no browser, system PDF, system font, Cairo/Pango, or AI-vendor dependency.
- Wheels retain `cp310-abi3` compatibility for CPython 3.10 through 3.14 across the 15-platform release matrix.
- Rust 1.85 remains the minimum supported toolchain.
- Packages: `fullbleed 2.3.1` on crates.io/PyPI; unchanged `fullbleed_audit_contract 0.1.3` on crates.io and `fullbleed-mcp 0.1.0` on PyPI.

## Release gates

- Locked default and `python,svg_raster` Rust suites, including regressions for Type 3 preview metadata and registered-font fixed bindings.
- Complete Python suite against the installed 2.3.1 ABI3 wheel.
- Installed-wheel smoke, generated agent-contract drift, license, release-metadata, package-file, MSRV consumer, Skill-format, and deterministic-output gates.
- The retained diagnostic scenarios verify readable synthetic weights and resolved custom-font VDP values in finalized previews; package claims remain limited to checks actually run.

Exact cross-platform artifact hashes and installed-wheel results are retained by the final release workflow.
