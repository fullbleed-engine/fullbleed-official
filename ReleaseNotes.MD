# Fullbleed Release Notes - 2.5.15

Overlapping `border-image-slice` values now preserve the corner image content.
Each source slice is clamped independently; empty intervening regions leave
gaps instead of shrinking every slice. Uniform-gradient shortcuts also respect
those gaps. Proportional reduction of oversized destination widths is unchanged.

Finalized-PDF previews now apply a transparency group's soft mask and parent
alpha once at the group boundary. This corrects missing gaps in masked forms;
the PDF writer already retained the mask. Four renderer regressions cover path
gaps, shading, overlapping members, and the mask transform with caller clipping.

The installed-wheel checks cover 21 gradient, PNG, SVG, overlap, clamp, fill,
destination-width and mask-border cases in native previews, finalized-PDF
previews and independent PDFium rendering. Release CI retains the PDFs and all
three preview modes. See the [2.5.15 runbook](docs/release/2.5.15-runbook.md).

The full corpus comparison of source commit `9ec4f71`, before this version bump,
changed one case from FAIL to PASS. The other 1,661 PDFs were byte-identical to
the published 2.5.14 baseline. That source run has 1,617 PASS, 26 FAIL and 19
REFERENCE-DISPUTED results; its upstream gate remains failed. These are results
for that retained source candidate, not a fresh registry-wheel corpus run or a
claim of full CSS parity. The published 2.5.14 evidence remains unchanged.

---

# Fullbleed Release Notes - 2.5.14

Arabic text with combining marks could duplicate its base letters when copied
from a PDF. Internal direction wrappers could also replace ordinary spaces in
the PDF Unicode map. The map now distinguishes combining marks from their base
cluster and omits internal direction-wrapper glyphs.

Styled inline text now resolves bidirectional levels across its containing
line. Arabic labels, Latin identifiers and dates retain the surrounding text
context; explicit isolation keeps a field's contents together through nested
styles. RTL inline words participate in line wrapping, and inline blocks remain
atomic objects in the surrounding bidi paragraph.

Standalone `unicode-bidi` rules are no longer discarded as empty declarations.
HTML direction attributes and bidi elements receive default styling. Author
rules take precedence over user-agent defaults, including within CSS layers.

The installed-wheel regression uses a pinned OFL Arabic font and independent
PDF readers to check marks, spaces, styled siblings, date isolation, nested
styles and narrow paragraphs. CI retains its PDFs, extracted text and previews.
PDF readers still differ in their reconstruction of mixed-direction lines;
these checks do not assert identical copy/paste behavior in every reader or
universal language support. See the [2.5.14 runbook](docs/release/2.5.14-runbook.md).

---

# Fullbleed Release Notes - 2.5.13

This is the published release of the tagged decoration, list numbering and
figure-caption corrections described below for 2.5.12. The 2.5.12 tag was
retained, but its registry workflows stopped before uploading packages because
the README's wheel example still named the previous version.

The wheel example now matches the release. Pull-request CI also runs the
release metadata check so future version mismatches are caught before tagging.
The engine implementation is identical to the tested 2.5.12 source.
See the [2.5.13 runbook](docs/release/2.5.13-runbook.md).

---

# Fullbleed Release Notes - 2.5.12

Release date: October 7, 2026

## Preserve decoration, list labels and figure captions in tagged output

A paragraph with differently sized inline text can emit a grouping tag with
tagged children. Its borders and backgrounds previously remained outside both
an MCID and an Artifact sequence, causing PDF/UA validation failures. Decorative
paint now receives artifact marking when no enclosing MCID covers it. Meaningful
figure paint keeps its existing structure, including within a grouping child.

Screen-reader-only ActualText also closes an automatic decorative artifact
before its own marked-content sequence begins.

Lists now emit ListNumbering attributes from their rendered marker styles,
including item overrides, nested lists and paginated fragments. Generic marker
kinds and description lists use PDF 2.0 attributes only in PDF 2.0 profiles.
Single-image figures retain their caption within the semantic Figure; the
image alternative describes that group without being announced twice.

The regression checks inspect emitted PDF operators and structure trees across ordinary, compiled,
repeated and reflow rendering with tagged, PDF/UA-1 and PDF/UA-2 profiles.
The release gate also validates a richer specimen with decorated inline text,
table headers, nested lists and captions before and after illustrations.
The corrections change affected PDF structure; it is not a blanket accessibility
or conformance claim. Keep reviewing each final document and its validation
reports. See the [2.5.12 runbook](docs/release/2.5.12-runbook.md).

---

# Fullbleed Release Notes - 2.5.11

Release date: October 6, 2026

## Preview standard PDF fonts without system fonts

An ordinary PDF using unembedded Helvetica, Times, Courier, Symbol, or Dingbats
could produce blank native previews on a host without suitable fonts. The native
renderer now uses compact, bundled OFL outline substitutes for these 14 faces.
Registered and embedded font programs retain precedence. The PDF font resources,
text and advances remain unchanged; default preview pixels intentionally change.

The substitutes total 508,020 bytes and cover every glyph in the pinned AFMs'
built-in encodings. The unencoded, vendor-private Symbol `/apple` logo is not
available. These are substitute designs, not original Standard 14 outlines.
Sources, reproducible generation, hashes and complete license notices are retained.

Font-resolution traces report `bundled_substitute` with a deterministic bundled
source, and runtime discovery adds `bundled_standard_font_previews`. Missing
requested fonts still retain their existing fallback reason. No runtime package,
browser, network download, or system-font installation is introduced.

The installed-wheel regression checks 68 cases, including all encoded glyphs,
named AFM glyphs, explicit-font overrides and embedded-font controls. CI retains
the PDFs and previews and compares Linux output with system fonts present and
hidden. Local Windows and isolated Linux results also match pixel-for-pixel.

See the [2.5.11 release runbook](docs/release/2.5.11-runbook.md). Node and .NET
packages have separate releases; their current versions still use engine 2.5.10.

---

# Fullbleed Release Notes - 2.5.10

Release date: October 6, 2026

## Keep inline labels at their intended width

The 2.5.9 inline wrapping correction exposed word fragments to the shared line
builder. Intrinsic sizing did not include the spacing and layout-unit rounding
between those fragments. A flex or inline-block label could consequently wrap
inside its own max-content width even when the page had enough room.

Intrinsic widths now reserve the same advances used by inline layout. Min-content
sizing also recognizes collapsed-space word boundaries, while no-wrap runs keep
their combined width. Leading and trailing collapsed spaces do not inflate it.
This restores tracked labels and footers without reverting the styled-inline
overlap correction. Affected PDFs can change line breaks and hashes; review saved
baselines when upgrading.

The installed-wheel checks retain independent word coordinates and previews for
six flex/inline-block specimens, alongside the existing 48-case wrapping suite.
This introduces no public API or runtime dependency and is not a claim of full
CSS or PDF standards conformance. Node and .NET packages have separate releases.

See the [2.5.10 release runbook](docs/release/2.5.10-runbook.md).

---

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
