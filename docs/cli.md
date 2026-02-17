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
- `finalize` (template composition workflow)
- `inspect` (PDF metadata + compatibility inspection)

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
- template compose (auto-finalize): `--template-binding`, `--templates`, `--template-dx`, `--template-dy`
- page/pdf: `--page-size`, `--page-width`, `--page-height`, `--margin`, `--pdf-version`, `--pdf-profile`
- diagnostics: `--emit-jit`, `--emit-perf`, `--emit-glyph-report`, `--emit-page-data`, `--emit-compose-plan`
- image artifacts: `--emit-image`, `--image-dpi`
- policy: `--profile`, `--fail-on`, `--allow-fallbacks`, budget flags
- reproducibility: `--deterministic-hash`, `--repro-record`, `--repro-check`

Template auto-compose notes:
- When `--templates` is set on `render`, CLI renders overlay, resolves template bindings, and finalizes via Rust compose in one command.
- Requires `--template-binding` and file output (`--out` cannot be `-`).
- When `--emit-image` is used with template auto-compose, image artifacts are emitted from the finalized composed PDF (not overlay-only preview) via native Rust rasterization in the engine.
- `--deterministic-hash` writes PDF SHA-256 by default; when `--emit-image` is set, it writes an artifact-set digest (`fullbleed.artifact_digest.v1`) computed from PDF SHA-256 plus ordered page-image SHA-256 values.

## `verify`

Same pipeline as render but tuned for validation/preflight usage. Can emit PDF optionally with `--emit-pdf`.

## `plan`

Generates normalized compile manifest (`fullbleed.compiler_input.v1`) and warnings (for example remote refs without allow flag).

Use `--emit-manifest <path>` to persist manifest JSON.

Template composition planning:
- with `--templates` + `--template-binding`, `plan` resolves template bindings and compose plan rows.
- use `--emit-compose-plan <path>` to write `fullbleed.compose_plan.v1`.

## `run`

Runs a Python entrypoint and renders with that returned engine:

```bash
fullbleed run report:engine --html input.html --css styles.css --out out.pdf
```

Entrypoint can be `module:name` or `path/to/file.py:name`.

## `finalize`

Template composition command group:

- `fullbleed finalize stamp --template <template.pdf> --overlay <overlay.pdf> --out <final.pdf>`
- `fullbleed finalize compose --templates <dir> --plan <plan.json> --overlay <overlay.pdf> --out <final.pdf>`
- Stamp placement controls: `--dx <pt> --dy <pt>` for explicit overlay translation when needed.

Current state:
- `stamp` is implemented through the Rust core finalize path with strict checks and JSON result envelope
- `compose` is implemented as a Rust-backed baseline with strict plan/catalog validation

## `inspect`

Inspection command group:

- `fullbleed inspect pdf <path> [--json]`
- `fullbleed inspect pdf-batch <path...> [--list paths.txt] [--json]`
- `fullbleed inspect templates --templates <dir|json> [--json]`

Use this to read canonical PDF metadata from the Rust inspector without rendering:

- `pdf_version`
- `page_count`
- `encrypted`
- `file_size_bytes`
- composition compatibility (`supported`, `issues`)

Schema target:

- `fullbleed.inspect_pdf.v1`
- `fullbleed.inspect_pdf_batch.v1`
- `fullbleed.inspect_templates.v1`

## `new`

Template/project bootstrap command group:

- Local templates:
  - `fullbleed new local invoice <path>`
  - `fullbleed new local statement <path>`
  - Compatibility aliases are still supported:
    - `fullbleed new invoice <path>`
    - `fullbleed new statement <path>`
- Remote registry:
  - `fullbleed new list [--registry <manifest-url>]`
  - `fullbleed new search <query> [--tag <tag>] [--registry <manifest-url>]`
  - `fullbleed new remote <template_id> [path] [--version latest|<x.y.z>] [--registry <manifest-url>]`

Practical notes:
- Default registry URL can be overridden with `--registry` or `FULLBLEED_TEMPLATE_REGISTRY`.
- `new remote --dry-run` resolves template/release metadata without downloading archives.
- Remote install verifies archive SHA256 before extraction and blocks path traversal in zip contents.

## Machine-mode schemas

`--schema` returns:

- envelope schema: `fullbleed.schema.v1`
- inferred target schema for the command/subcommand
- schema definition when available

Examples:

```bash
fullbleed --schema render
fullbleed --schema assets list
fullbleed --schema inspect templates
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
- `fullbleed assets install inter`
- `fullbleed assets lock`
- `fullbleed cache dir`
- `fullbleed cache prune --dry-run`

Note:
- `@noto-sans` remains available for broader glyph coverage, but it has a larger font payload and should be installed intentionally.

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
