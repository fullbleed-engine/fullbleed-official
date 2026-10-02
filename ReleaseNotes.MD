# Fullbleed Release Notes - 2.5.2

Release date: October 2, 2026

## Clearer inputs for document agents

The MCP tools now explain every input parameter, including source selection,
workspace paths, preview resolution, PDF profile prerequisites, and asset action
requirements. Compiled-document tools describe columnar bindings, static versus
reflow output, handle reuse, and handle expiration. Verification guidance makes
the selected failure checks explicit.

Tool descriptions distinguish rendering, previewing, document verification,
existing-PDF inspection, and asset integrity checks. The asset action is correctly
named `info`. Side-effect hints acknowledge file replacement, compiled-handle
eviction, and possible network access when installing remote asset packages.
These hints describe existing behavior; they do not grant permissions or change
tool execution.

The shared runtime definitions supply the generated agent contract and both MCP
entrypoints. Input and output validation constraints retain their existing shapes.
This release adds no core runtime dependency and makes no rendering algorithm
changes. The separately versioned MCP adapter remains 0.1.1.

Upgrade the engine in the Python environment used by your MCP client, then restart
that client:

```sh
python -m pip install --upgrade fullbleed fullbleed-mcp
```

The [agent setup guide](https://docs.fullbleed.dev/guides/ai-agents/) covers local
MCP configuration and installing the Agent Skill from a pinned GitHub release.
See the [2.5.2 release runbook](docs/release/2.5.2-runbook.md) for the required
verification and publication checks. Tool descriptions and profile selection do
not establish PDF standards conformance.

---

# Fullbleed Release Notes - 2.5.1

Release date: October 1, 2026

## MCP client compatibility

MCP tool discovery now declares `"type": "object"` at the root of every tool's
output schema. Earlier schemas already described object-valued success and
error results through `anyOf`, but strict clients rejected `tools/list` because
the required top-level type was missing.

The correction lives in the shared runtime tool definitions. Generated agent
contracts and both MCP entrypoints therefore describe the same schemas. Tool
names, inputs, structured results, and error variants keep their existing shape.

Upgrade the engine in the Python environment used by your MCP client:

```sh
python -m pip install --upgrade fullbleed fullbleed-mcp
```

The release adds an independent interoperability check using the official
TypeScript MCP SDK. It exercises discovery, a structured capabilities result,
PDF preview generation, and a structured tool error against the installed wheel.
The SDK is a development-only test dependency; the core Python package gains
no runtime dependency. The Rust renderer is unchanged.

See the [2.5.1 release runbook](docs/release/2.5.1-runbook.md) for required checks
and the evidence retained before publication.

---

# Fullbleed Release Notes - 2.5.0

Release date: October 1, 2026

## Rebuild PDFs while editing

`fullbleed render --watch` renders immediately, then rebuilds when explicit
HTML, CSS, asset, or file-backed configuration inputs change. Polling uses
Python's standard library and adds no runtime dependency.

```sh
python -m fullbleed render --html invoice.html --css styles.css --asset "@noto-sans" --out invoice.pdf --emit-image preview --watch
```

- Add files or recursive directories with repeatable `--watch-path` options.
- Editor save bursts are debounced; edits made during rendering trigger another pass.
- Missing inputs and failed render gates produce ordinary CLI errors while the watcher remains ready for the next edit.
- CLI output files and the preview directory are excluded from scans. Obsolete previews from this session are removed only if they remain unmodified.
- `--json-only` streams the existing render-result and error schemas as newline-delimited JSON. Successful results include the PDF SHA-256; requested manifests refresh each cycle.
- Stop with Ctrl-C. The polling interval and debounce duration are configurable.

The [watch guide](docs/render-watch.md) and
[complete invoice example](examples/render_watch) explain the workflow.
Fullbleed writes files; it does not launch or refresh a viewer, discover every
dependency referenced inside source files, or rerun Python generators. Watch
mode requires local input dependencies and file output. Use a one-shot render
or verify command for delivery gates.

## Examples and discovery

- A runnable first-invoice notebook includes a Colab link and a generated PDF preview.
- FastAPI, Flask, and Django examples serve the same invoice with shared rendering code and verified HTTP response headers.
- The [designed showcase](https://docs.fullbleed.dev/examples/) includes an editorial invoice, a vector-illustrated report, a tagged notice, and compiled member statements, with complete HTML/CSS, data, and licensed font sources. Its published PDFs remain reproducible with the public 2.4.0 wheel.
- The generated agent contract and CLI reference expose watch-mode flags, result streaming, and scope limits from the installed runtime.

## Compatibility and verification

- Ordinary one-shot rendering retains its existing behavior. No Rust rendering changes or new core Python dependencies are introduced by this release.
- The release matrix remains 15 CPython ABI3 wheels, supporting CPython 3.10–3.14. Rust MSRV remains 1.85; the audit crate remains 0.1.3 and the MCP adapter remains 0.1.0.
- Watch behavior is checked with an installed CLI process, including CSS edits, deleted-input recovery, recursive dependencies, stale-preview cleanup, and output exclusion. CI retains generated PDFs, previews, and JSON events. Native release-wheel checks also cover macOS and Windows architectures.
- Existing print-profile release gates remain required. Full ISO PDF/VT-1 conformance has not been independently established; this release does not claim certification or exhaustive ISO coverage.

See the [2.5.0 release runbook](docs/release/2.5.0-runbook.md) for the required
checks and retained-evidence procedure.
