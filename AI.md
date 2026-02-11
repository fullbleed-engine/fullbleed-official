<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# Fullbleed Agent Guide

This document is for AI agents using `fullbleed` in real user environments.

Scope:
- Assumes a normal installed release (`pip install fullbleed`), not a mixed local dev tree.
- Focuses on reliable automation patterns for repeatable document generation.
- Treat Fullbleed as a document engine, not a web-to-print/browser-runtime substitute.
- Treat HTML/CSS as the familiar document DSL; with pinned assets and flags, Fullbleed targets reproducible outputs.

## Core Rule

Use `fullbleed init` as the default path for project work.

Why:
- It creates project markers (`report.py`, `assets.lock.json`, `fullbleed.toml` patterns) that enable project-aware behavior.
- Asset installs default to project vendoring (`./vendor/...`) when markers are present.
- It supports repeatable runs, lock files, and easier compliance review.
- It prevents brittle assumptions about global cache layout.

Use direct one-off CLI rendering only for quick ad hoc jobs.

## Happy Path (Project)

1. Install:
```bash
python -m pip install fullbleed
```

If `fullbleed` is not on `PATH`, use:
```bash
python -m fullbleed --help
```

2. Initialize project:
```bash
fullbleed init .
```

`init` vendors a predictable baseline by default:
- `vendor/css/bootstrap.min.css` (+ `LICENSE.bootstrap.txt`)
- `vendor/fonts/Inter-Variable.ttf` (+ `LICENSE.inter.txt`)
- seeded `assets.lock.json` entries with pinned hashes

3. Install additional assets:
```bash
fullbleed assets install inter --json
```

4. Render:
```bash
fullbleed --json render --html templates/report.html --css templates/report.css --out output/report.pdf
```

5. Verify and lock:
```bash
fullbleed assets verify bootstrap --json
fullbleed assets lock --add inter --json
```

## Agent Operating Rules

1. Always prefer machine mode.
- Use `--json` (or `--json-only`) and parse output.
- Validate `schema` and `ok` fields.

2. Never hardcode cache paths.
- Do not construct paths under `%LOCALAPPDATA%/fullbleed/cache/...`.
- Use `assets install --json` and consume `installed_to`.

3. Do not assume install succeeded.
- Check process exit code.
- Check JSON `ok`.
- Stop on `fullbleed.error.v1` responses.

Exit-code handling contract:
- `0`: success, parse JSON payload if requested.
- `1` or `3`: command/runtime error, prefer JSON parse; fall back to stderr/stdout text.
- `2`: argparse usage error, expect usage text (not JSON).

4. Use stable builtin package names.
- Builtins accept both forms:
  - `bootstrap` and `@bootstrap`
  - `noto-sans` and `@noto-sans`
- In PowerShell, quote `@...` aliases (for example `"@bootstrap"`).

5. For repeatable/production workflows, prefer project vendoring.
- Use `fullbleed init` first.
- If not in a project, installs may go to global cache.

6. Concurrency claims: keep them precise.
- Python render calls release the GIL while Rust executes.
- Rayon is used for explicit parallel batch APIs and selected internal engine workloads.
- Do not claim that every single-document render fully uses all cores.

7. For transactional workflows, run confidence checks for headers, footers, and totals.
- Use the minimal transactional example in `README.md` as the baseline fixture pattern.
- Validate machine semantics with `render_pdf_with_page_data(...)` and assert:
  - `totals["items.amount"]["value"] == sum(page["items.amount"]["value"] for page in pages)`.
- Validate visible behavior by text extraction from the produced PDF:
  - page 2 contains `Statement Continued` and `Subtotal (Page 2):`
  - last page contains `Grand Total:`

8. For per-page template layouts, use numeric + `n` mapping (not ad-hoc key names).
- Treat docs terms `page_1`, `page_2`, `page_n` as:
  - `1`, `2`, `n` in `page_margins`.
- For CLI, pass `--page-margins` JSON with keys `1`, `2`, `n`.
- For Python, pass `page_margins={1: ..., 2: ..., "n": ...}`.

9. If the user states they are on a commercial license, attest it in tooling context.
- CLI path:
  - `fullbleed compliance --license-mode commercial --commercial-license-id <id> --json`
- Env path (automation):
  - `FULLBLEED_LICENSE_MODE=commercial`
  - `FULLBLEED_COMMERCIAL_LICENSED=1`
  - `FULLBLEED_COMMERCIAL_LICENSE_ID=<id>`
- Python path:
  - `fullbleed.activate_commercial_license(<id>, company=..., tier=...)`

10. For SVG outputs, use the explicit SVG paths instead of guessing.
- Standalone SVG document:
  - `fullbleed --json render --html artwork/badge.svg --out output/badge.pdf`
- Inline SVG:
  - `fullbleed --json render --html-str "<svg ...>...</svg>" --out output/inline.pdf`
- HTML + external SVG asset:
  - `fullbleed --json render --html templates/report.html --css templates/report.css --asset assets/logo.svg --asset-kind svg --out output/report.pdf`
- Use `fullbleed capabilities --json` and check the `svg` object for machine-readable SVG support metadata.

## Recommended Defaults for Automation

- `--json-only` for command outputs consumed by agents.
- `--pdf-version 1.7` for production-stable output.
- `--repro-record <path>` during baseline generation.
- `--repro-check <path>` in reruns/CI.
- `fullbleed doctor --strict --json` in pipeline preflight.
- `fullbleed compliance --strict --json` in release checks.

## One-off Mode (Allowed)

For quick one-time outputs, this is valid:
```bash
fullbleed --json render --html-str "<h1>Hello</h1>" --css-str "body{font-family:sans-serif}" --out output/hello.pdf
```

But for any reusable workflow, switch to the project path (`init` + vendored assets + lock file).

## Why This Matters

The biggest real-world agent failure mode is not rendering itself. It is path and state assumptions:
- assuming assets are present without verifying,
- assuming cache layout,
- assuming install success.

`fullbleed init` plus JSON-driven asset installation removes that class of errors and yields deterministic, auditable project behavior.
