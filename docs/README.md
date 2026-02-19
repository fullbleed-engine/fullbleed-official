# Fullbleed Documentation

This folder documents the Fullbleed stack at three layers:

1. Engine internals and render pipeline
2. Python API surface (`import fullbleed`)
3. CLI surface (`fullbleed ...`)

## Version scope

These docs target the `0.2.7` stable line and the current repository source layout.

## Documents

- `docs/engine.md`: Rust engine architecture, render flow, pagination model, diagnostics
- `docs/python-api.md`: Python bindings, classes, methods, and usage patterns
- `docs/cli.md`: command reference, JSON/machine mode, reproducibility and validation flows
- `docs/pdf-templates.md`: Rust finalize PDF template/XObject composition policy and smoke gates

## Recommended reading order

1. `docs/python-api.md` if you are building reports/components in Python
2. `docs/cli.md` if you are automating builds/validation in CI
3. `docs/pdf-templates.md` if your workflow overlays variable data onto source PDF templates
4. `docs/engine.md` if you need to reason about behavior, constraints, or performance

## Scaffold and component workflow

For component-first project structure and scaffold conventions, read:

- `python/fullbleed_cli/scaffold_templates/init/SCAFFOLDING.md`
