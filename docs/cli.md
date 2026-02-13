# CLI Reference

Executable entrypoint:

- `fullbleed`

Python module entrypoints:

- `python -m fullbleed_cli`
- `python -m fullbleed` (delegates to CLI)

## Global flags

- `--json`: emit JSON result payloads
- `--json-only`: strict machine mode (`--json`, no prompts)
- `--schema`: print schema envelope for the requested command
- `--config`
- `--log-level`
- `--no-color`
- `--no-prompts`

## Command groups

Core render pipeline:

- `render`
- `verify`
- `plan`
- `run`

Diagnostics and introspection:

- `doctor`
- `capabilities`
- `compliance`
- `debug-perf`
- `debug-jit`

Asset/cache:

- `assets list|info|install|verify|lock`
- `cache dir|prune`

Project generation:

- `init`
- `new`

## Core workflow commands

## `render`

Render HTML/CSS to PDF and optionally emit diagnostics/artifacts.

High-value options:

- input: `--html` / `--html-str`, `--css`, `--css-str`
- output: `--out`
- assets: `--asset`, `--asset-kind`, `--asset-name`, `--asset-trusted`
- page/pdf: `--page-size`, `--page-width`, `--page-height`, `--margin`, `--pdf-version`, `--pdf-profile`
- diagnostics: `--emit-jit`, `--emit-perf`, `--emit-glyph-report`, `--emit-page-data`
- image artifacts: `--emit-image`, `--image-dpi`
- policy: `--profile`, `--fail-on`, `--allow-fallbacks`, budget flags
- reproducibility: `--deterministic-hash`, `--repro-record`, `--repro-check`

## `verify`

Same pipeline as render but tuned for validation/preflight usage. Can emit PDF optionally with `--emit-pdf`.

## `plan`

Generates normalized compile manifest (`fullbleed.compiler_input.v1`) and warnings (for example remote refs without allow flag).

Use `--emit-manifest <path>` to persist manifest JSON.

## `run`

Runs a Python entrypoint and renders with that returned engine:

```bash
fullbleed run report:engine --html input.html --css styles.css --out out.pdf
```

Entrypoint can be `module:name` or `path/to/file.py:name`.

## Machine-mode schemas

`--schema` returns:

- envelope schema: `fullbleed.schema.v1`
- inferred target schema for the command/subcommand
- schema definition when available

Examples:

```bash
fullbleed --schema render
fullbleed --schema assets list
```

## Fail-on policy

Supported checks:

- `overflow`
- `missing-glyphs`
- `font-subst`
- `budget`

Budget limits:

- `--budget-max-pages`
- `--budget-max-bytes`
- `--budget-max-ms`

Set `--allow-fallbacks` to permit fallback-related signals without failing.

## Assets and cache commands

Use:

- `fullbleed assets install @bootstrap`
- `fullbleed assets install @noto-sans`
- `fullbleed assets lock`
- `fullbleed cache dir`
- `fullbleed cache prune --dry-run`

Project-aware installs default to `./vendor/`; use `--global` for cache install behavior.

## Compliance command

`fullbleed compliance` emits a policy report (`fullbleed.compliance.v1`) including:

- licensing file checks
- third-party notice checks
- audit artifact staleness checks
- commercial attestation metadata status

Use `--strict` for non-zero exit on flags.

## Recommended CI usage

1. Use `--json-only` outputs
2. Enable `--fail-on` checks for your quality gates
3. Emit deterministic and repro artifacts
4. Parse command schema ids and output contracts in CI tooling

