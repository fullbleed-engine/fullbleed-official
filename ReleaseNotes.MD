# Fullbleed Release Notes - 2.5.9

Release date: October 6, 2026

## Wrap ordinary text around styled inline content

A paragraph such as `Edit <code>print.css</code> to change the paper size...`
could place its wrapped text over the prefix. Mixed inline runs now expose
ordinary word boundaries to the shared line builder. Following text uses the
remaining line width, and collapsed spaces at wrapped line edges are discarded.
Line-box and background wrappers preserve space semantics; letter and word
spacing remain part of the advance.

Built-in proportional fonts use their bundled PDF advances for inline word
fragments, so separating a styled run does not introduce excessive word spacing.

The change covers horizontal, left-to-right text with collapsed whitespace.
Preserved whitespace, no-wrap, vertical writing, and bidi runs retain their
paragraph path. It adds no runtime dependency or public API. Affected PDFs can
change line breaks, pagination, and hashes; review saved baselines when upgrading.

The internal PDF text extractor can return additional newlines between separately
positioned words. Normalize whitespace when checking complete phrases.

The installed-wheel regression checks direct PDFs, fixed bindings with an
explicitly wide enough variable slot, and both compiled reflow compression modes.
It retains native previews, independent PDFium previews, text extraction, and word
bounds. Fixed-template replacement streams are checked by visual position and
single-copy content, without claiming their stream order equals reading order.

See the [2.5.9 release runbook](docs/release/2.5.9-runbook.md). Node and .NET
packages have separate releases; this core release does not update them.

---

# Fullbleed Release Notes - 2.5.8

Release date: October 5, 2026

## Select the regular face for normal family text

Registering an italic face before its regular sibling previously made normal
family text use the italic font. Family aliases now prefer an upright face and
the CSS weight-400 fallback order. Both legacy and typographic family names are
recognized, and directory registration uses sorted paths. Face-specific names,
explicit source aliases, and `@font-face` mappings remain available; duplicate
explicit aliases and equally ranked family faces retain first-registration
priority.

Affected PDFs can change appearance, line breaks, and hashes. Review saved
baselines before accepting updates. The core adds no dependency or public API.
The installed-wheel check compares 54 cases / 84 pages with explicit-face
controls, inspecting embedded fonts, extracted text, and native/PDFium previews.
This is focused regression evidence, not complete CSS font matching conformance.

See [font registration](docs/font-registration.md) and the
[2.5.8 release runbook](docs/release/2.5.8-runbook.md).

---

# Fullbleed Release Notes - 2.5.7

Release date: October 5, 2026

## Smaller embedded TrueType subsets

TrueType subsets now omit unused character-map entries, glyph names, and metrics
while preserving the original glyph IDs, retained outlines and instructions,
composite dependencies, advances, and font notices. Horizontal-header extrema
are recalculated for the subset. Character-map formats other than 4 and 12 keep
their original bytes; an unsupported or larger map rewrite uses the source map.

The change adds no runtime dependency and does not change the rendering API.
PDF bytes and hashes change. Review stored PDF baselines before updating them.
Independent font, text, and pixel checks are described in the
[2.5.7 release runbook](docs/release/2.5.7-runbook.md). Claims remain limited to the
retained fixtures and validator reports.

---

# Fullbleed Release Notes - 2.5.6

Release date: October 3, 2026

## Keep synthesized bold text searchable once

PDFs using upright synthesized bold previously contained both visible Type 3
glyph text and an invisible logical text copy. Independent readers could extract
headings twice or interleave their letters. Visible glyphs now use shared vector
paint forms, leaving one logical text copy for search and extraction.

The finalized preview reader preserves the affine transform of those path forms
so outlines and strokes scale together. Selecting an unused fallback font after
a transformed element no longer causes an embedded-font profile to reject the
document. Fonts used by actual text still require embedding for those profiles.

Installed-wheel checks cover 36 styled fixtures across ordinary, tagged, and
PDF/UA-1-profile output, with retained PDF/PNG evidence and an independent text
reader in CI. Local comparisons use four text readers. These fixture checks do
not establish general renderer parity, screen-reader acceptance, or standards
conformance. The core gains no runtime dependency.

Affected PDF and finalized-preview hashes change. Review saved baselines before
updating them. See the [2.5.6 release runbook](docs/release/2.5.6-runbook.md).

---

# Fullbleed Release Notes - 2.5.5

Release date: October 2, 2026

## Keep gradients visible in finalized PDF previews

Native previews now paint linear and radial gradient fills from finalized PDFs.
Earlier versions ignored the PDF shading operator, leaving chart bars and other
gradient backgrounds blank even though a PDF viewer displayed them correctly.

The reader preserves stitched color stops, coordinate transforms, clipping, and
the gradient opacity masks emitted by Fullbleed. Form boundaries clip their
contents and keep their paths and graphics state separate from the caller.
Bounded function parsing rejects unsupported gradient programs explicitly.
The change adds no core runtime dependency.

The supported preview subset includes exponential and stitching functions for
axial and extended radial shadings, plus Fullbleed's generated hard radial bands.
Arbitrary calculator functions, mesh shadings, nonextended radial shadings, and
nonidentity mask transfer functions remain unsupported. These checks do not
establish general PDF rendering parity or standards conformance.

Affected PNG hashes change. Review corrected previews before updating stored
baselines. The retained authoring fixtures check that this preview fix leaves
their PDF bytes unchanged.

See the [2.5.5 release runbook](docs/release/2.5.5-runbook.md) for the installed-wheel
gradient checks, earlier-version negative control, and independent comparisons.

---

# Fullbleed Release Notes - 2.5.4

Release date: October 2, 2026

## Correct spacing in native Standard 14 previews

Native previews of finalized PDFs now use the standard advances for unembedded
Helvetica, Times, Courier, Symbol, and ZapfDingbats fonts when the PDF omits
`/Widths`. Earlier versions used a constant fallback advance, which could spread
narrow letters apart or overlap wide letters even when a separate PDF viewer
displayed the document correctly.

The preview reader respects built-in encodings and explicit encoding differences.
Explicit PDF widths and embedded replacement fonts keep their existing behavior.
The bundled metric and character tables add no runtime dependency. Their pinned
sources, generator, and redistribution notices are included in the source tree.

This patch changes native preview placement, so stored preview hashes may change
for affected PDFs. Review the new images before updating a reproducibility
baseline. PDF authoring and layout measurements are unchanged. Unembedded fonts
still use the preview renderer's available substitute outlines; the fix does not
promise identical outlines across PDF viewers.

See the [2.5.4 release runbook](docs/release/2.5.4-runbook.md) for regression checks
against explicit reference widths and the scope of retained verification.

---

# Fullbleed Release Notes - 2.5.3

Release date: October 2, 2026

## Reproducibility checks reject invalid baselines

`--repro-check` now rejects empty or malformed records, unsupported record schemas,
missing or invalid SHA-256 fields, and invalid asset-lock metadata. Earlier releases
could report success for `{}` or a record without hashes. Unreadable JSON and
non-object JSON return a structured `REPRO_RECORD_INVALID` failure.

A recorded asset lock now detects file removal, addition, and changed contents.
Using `--repro-record` and `--repro-check` on the same file, including an existing
hard-link alias, returns `REPRO_RECORD_CONFLICT` without replacing the baseline.
Different files remain supported so a failed check can retain a candidate record
alongside the changed PDF and previews for review.

Valid `fullbleed.repro_record.v1` baselines remain compatible. Unknown metadata
fields remain accepted. The installed record schema supplies the validation rules
and generated agent contract. This patch changes CLI checks, not rendering code,
and adds no core runtime dependency.

Upgrade before using reproducibility records as CI gates:

```sh
python -m pip install --upgrade fullbleed
```

See the [CLI reproducibility notes](docs/cli.md#reproducibility-checks) and
[2.5.3 release runbook](docs/release/2.5.3-runbook.md). A matching hash establishes
agreement with the recorded fixture, not visual correctness or PDF conformance.

---

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
