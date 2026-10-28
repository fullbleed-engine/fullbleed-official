# Fullbleed Release Notes - 2.5.41

Preformatted blocks now respect the author's CSS whitespace and wrapping
policy. For example, `pre { white-space: pre-wrap; overflow-wrap: anywhere; }`
wraps long code lines onto the available page width and continues them across
pages. Previously, the HTML `pre` handler forced no wrapping even when the
stylesheet requested it, so code could extend beyond the page edge.

Default `pre` and explicit no-wrap policies retain their existing behavior.
Direct, compiled and reflow output are covered by native regressions and
installed-wheel tests that check saved PDF text, visible glyph bounds and
whitespace. No runtime dependency or public API is added. The component alpha
remains experimental. See the [2.5.41 runbook](docs/release/2.5.41-runbook.md)
for the retained negative controls and release gates; these checks do not
establish general CSS parity or PDF conformance.

# Fullbleed Release Notes - 2.5.40

Underlines, overlines and strikethroughs now continue across interior word
spaces. Collapsed spaces at line edges remain unpainted. Nested inline text
keeps the ancestor's decoration, color and baseline; a child's
`text-decoration: none` no longer erases a line drawn by its ancestor. Atomic
inline boxes isolate inherited lines while preserving their own decorations.

This is a focused backport of the existing development-branch correction.
No new runtime dependency or public API is introduced. The component alpha
remains experimental. See the [2.5.40 runbook](docs/release/2.5.40-runbook.md)
for the stable negative control, installed-wheel checks and release gates.
These checks do not establish general CSS parity or PDF conformance.

# Fullbleed Release Notes - 2.5.39

Native previews now distinguish embedded font programs when several PDF pages
or forms use different subsets of the same font family. Previously, a preview
could borrow the first subset and omit or substitute glyphs from later subsets.
This affected template/overlay composition and PDFs assembled from independently
generated pages; the saved PDF could contain the correct text and still have an
incorrect PNG preview.

The preview renderer identifies each embedded program by its complete content
hash. Fullbleed's synthetic-bold Type 3 fonts resolve their source face within
their own resource scope. No new runtime dependency or public API is introduced.

See the [2.5.39 runbook](docs/release/2.5.39-runbook.md) for the negative control,
installed-wheel regressions, independent PDF inspection and release gates.
This correction does not establish general PDF raster parity or conformance.

# Fullbleed Release Notes - 2.5.38

CSS-generated quotation marks now follow document order across inline, block,
inline-block, and display-contents elements. Layout measurements no longer
consume quote depth or choose closing marks before a nested quotation is read.
Silent no-open-quote and no-close-quote tokens update nesting without drawing
text. Subtrees with `display: none` and elements without generated pseudo
children do not change the following quotation's depth. `visibility: hidden`
continues to participate in quotation nesting.

The correction applies to ordinary rendering, compiled templates, repeated
records, and reflowing records. Reusing an engine for another document starts
with fresh quote depth. Existing counter expressions retain their rendering
behavior. This is a focused backport of PR #135; no component-preview API or new
runtime dependency is included.

See the [2.5.38 runbook](docs/release/2.5.38-runbook.md) for validation gates and
scope. The prior development-branch timing checks were inconclusive; this change
does not claim improved throughput or general CSS parity.

# Fullbleed Release Notes - 2.5.37

Package descriptions now lead with the HTML/CSS workflow, keep component
authoring available below the language quickstarts, and use release-scoped
repository links that work from a package index. JavaScript and .NET packages
release independently; their guides supply tested version pins without leaving
obsolete install commands in the Python package description.

Release automation renders the descriptions from the actual wheels and source
distribution with PyPA's README renderer. It checks link/image destinations,
local anchors, and agreement across the distribution set before publishing.
The checker and its Markdown dependency are release tooling only.

The rendering engine and Python API source are unchanged from 2.5.36. This
release does not promote the component alpha or add a new conformance claim.
See the [2.5.37 runbook](docs/release/2.5.37-runbook.md).

# Fullbleed Release Notes - 2.5.36

Auto-layout tables now account for permitted text breaks when choosing column
widths. Long Japanese text can wrap within a specified table width without
manual `<br>` elements. Plain cells honor `word-break: break-all`,
`line-break: anywhere`, and `overflow-wrap: anywhere` during minimum-width
measurement. Cells containing inline or block elements use their wrappable
minimum instead of requiring the entire unwrapped line to fit.

Wrapped inline text inside cells also starts at the first line's baseline,
preventing earlier lines from being positioned above the row in both auto and
fixed table layouts.

Authored child widths, minimum widths, and box sizing remain part of a cell's
minimum contribution, including through nested elements. Percentage widths
that depend on the column do not create a circular minimum-size requirement.

Paragraph minimum-width measurement uses its existing CJK punctuation rules.
Unbreakable Latin text, `word-break: keep-all`, and `white-space: nowrap` retain
their minimum width; emergency `overflow-wrap: break-word` does not shrink it.
No browser, system font, or new runtime dependency is added.

See the [2.5.36 runbook](docs/release/2.5.36-runbook.md) for required evidence and
scope. This correction does not establish general CSS or CJK typography parity.

# Fullbleed Release Notes - 2.5.35

Long tables and auto-height containers with clipping overflow now continue onto
subsequent PDF pages. Previously, `overflow: hidden` could keep an unconstrained
container on one page and hide later rows. The correction preserves clipping
while allowing ordinary pagination, repeated table headers, descendant page
breaks, and footnote placement.

Explicit height, maximum height, line clamping, and aspect-ratio viewports retain
their bounded behavior. This stable backport does not include the component
alpha's percentage-column allocator changes or its authoring APIs.

See the [2.5.35 runbook](docs/release/2.5.35-runbook.md) for release gates and
verification scope. A passing fixture does not establish document-wide
accessibility or general CSS parity.

# Fullbleed Release Notes - 2.5.34

Compiled reflow now refreshes running headers and footers when record values
change. A binding shared by ordinary body text and a captured running element
could previously update the body while retaining the preceding record's header.
This corrects the reproduced behavior in [issue #104](https://github.com/fullbleed-engine/fullbleed-official/issues/104).

Generated page-margin text and decoration now share balanced artifact marking,
keeping page numbers and repeated navigation outside the body reading order in
tagged output. This addresses [issue #103](https://github.com/fullbleed-engine/fullbleed-official/issues/103).

Documents with running content reuse compiled layout only for identical binding
values. Changing body values also triggers reflow when the header itself is
static, reducing cache reuse. The choice preserves correct record content;
dependency-aware reuse can be optimized separately.

The stable correction retains existing page-margin geometry. The proposed
component page-band API remains in development. CI and release jobs retain
saved-PDF regressions for page furniture and running record values.

See the [2.5.34 runbook](docs/release/2.5.34-runbook.md) for verification scope
and release gates. Fixture checks do not establish accessibility for arbitrary
authored documents.

# Fullbleed Release Notes - 2.5.33

Table captions now remain inside their logical Table in tagged PDF output,
with one Caption group containing the caption paragraph. Top and bottom
captions retain their reading order across page continuations, nested tables,
compiled rendering and separate records. Empty or hidden captions retain
their layout without adding empty caption tags.

This stable update carries the engine correction from the 2.6.0a7 preview.
The experimental component API remains in the separate preview release.
CI and release builds now retain the saved-PDF caption regression matrix.

See the [2.5.33 runbook](docs/release/2.5.33-runbook.md) for validation scope and
release evidence. Profile selection alone does not establish accessibility
conformance for an authored document.

# Fullbleed Release Notes - 2.5.32

Oversized table cells can continue their text, paragraphs and lists across pages
with their repeated headers. The splitter handles top and default baseline
alignment, keeps column widths stable, and leaves content with ending cell
decoration instead of emitting an empty trailing page. This addresses the
repeated-header stall in [issue #99](https://github.com/fullbleed-engine/fullbleed-official/issues/99).
Continued cells retain one logical TH/TD and their explicit header references.

Compiled reflow also preserves embedded font subsets when replaying cached pages.
The replay cache is scoped to the actual batch values. Values whose visible text
uses separate glyph outlines require layout when changed; updating only their
searchable text could previously leave the first recipient's name visible.
Changed table values also require layout so that columns and other cells follow
the current record's content, including tables marked as presentation.
Independent PDFium checks compare each record with a separately rendered PDF,
including regular and synthesized-bold text, table geometry and both compression modes.

The new 240-case output gate checks visible ink, page bounds, content counts,
logical cells, header links, tagged/untagged pixels and repeated-render bytes.
Existing caption limitations in [issue #98](https://github.com/fullbleed-engine/fullbleed-official/issues/98)
remain separate. This does not establish general PDF/UA conformance or support
for every oversized rowspan/alignment combination. The experimental component
release requires its own qualification. No runtime dependency is added.

See the [2.5.32 runbook](docs/release/2.5.32-runbook.md) for evidence and release gates.

---

# Fullbleed Release Notes - 2.5.31

Rich table cells now keep their TH/TD structure separate from independently
tagged text, lists, images and nested tables. Previously, a cell containing a
paragraph or styled span could produce nested marked-content IDs even when a
PDF/UA validator reported a passing profile result. Table borders and
backgrounds are marked as artifacts. Plain cells retain their existing text
owner; untagged native content inside rich cells receives a content owner.

The saved-PDF regression gate covers 180 cases across tagged PDF, PDF/UA-1 and
PDF/UA-2, using direct, compiled, repeated and reflow output with both compression
modes. It checks marked-content nesting, structure and parent-tree ownership,
header scope, row/column spans, explicit header references, alternative text,
pagination, text retention and repeated-render determinism. CI retains the PDFs
and independent previews. This corrects [issue #97](https://github.com/fullbleed-engine/fullbleed-official/issues/97).

Existing PDF/UA-2 table-caption containment failures remain tracked in
[issue #98](https://github.com/fullbleed-engine/fullbleed-official/issues/98).
The new table gate is a focused structural regression check, not proof that all
PDF/UA documents are conformant. A pre-existing oversized rich-cell pagination
stall remains tracked in [issue #99](https://github.com/fullbleed-engine/fullbleed-official/issues/99);
the passing pagination fixtures split between rows. No runtime dependency or
public authoring API is added. The experimental component library remains a
separate opt-in release.

See the [2.5.31 runbook](docs/release/2.5.31-runbook.md) for validation evidence
and publication gates.

---

# Fullbleed Release Notes - 2.5.30

Same-font colored spans and generated content now preserve authored row spacing
at `line-height: 1`. Previously, affected font sizes added one CSS pixel to each
row: 24 rows that fit on three pages as plain text could become four pages after
adding an accent color. Direct rendering and compiled reflow use the corrected
logical line box. Registered fonts with integral CSS-pixel sizes and unit line
height use nearest-pixel descent. Logical line geometry retains its half-pixel
phase until painting; mixed-size text encloses the parent strut without changing
table, page-margin or atomic-inline baselines. Floated initials keep their
existing glyph placement, and text shadows preserve the foreground baseline.

The installed-output gate checks 380 cases by reading saved PDFs with PDFium:
absolute baselines at integral and fractional origins, row spacing, pagination
and mixed-size border geometry.
The public 2.5.29 wheel fails 22 of the original 64 spacing cases. The expanded
gate retains fresh browser measurements with pinned fonts. The existing 20
floated-initial references and tolerances remain unchanged. Runtime dependencies
and authoring APIs are unchanged.

Remote-template help now points to `fullbleed new list` so users can discover
available starters instead of copying an unavailable template ID.

See the [2.5.30 runbook](docs/release/2.5.30-runbook.md) for the independent corpus
review and final publication gates. Remaining fractional line-height paint
differences are outside this correction. The static Bootstrap component alpha
remains a separate opt-in release.

---

# Fullbleed Release Notes - 2.5.29

Native previews of finalized PDFs now composite isolated transparency groups
before applying their opacity. Overlapping SVG shapes and reused symbols no
longer become darker in PNG previews than in the generated PDF. Member opacity,
stroke opacity and nested group opacity remain distinct. Runtime dependencies
and the document authoring APIs are unchanged.

The SVG output checks now inspect both layout previews and previews read from
the saved PDF, across direct, compiled, fixed-binding and reflow output. The
previous public wheel fails 16 finalized-preview checks while its PDFium-rendered
PDFs remain correct. The candidate must pass all 810 checks across 54 inputs and
preserve the retained PDF bytes and layout-preview pixels before release.

See the [2.5.29 runbook](docs/release/2.5.29-runbook.md) for evidence and publication
gates. This is a scoped correction for isolated groups emitted by Fullbleed;
it does not establish general external-PDF renderer or CSS conformance. The
separate static Bootstrap component alpha remains an explicit opt-in release.

---

# Fullbleed Release Notes - 2.5.28

Inset box shadows now preserve negative spread, large offsets and translated
rounded corners. Table cells share the corrected geometry. Zero-blur shadows
remain vector, and unshadowed edges no longer acquire faint clipping fringes.
Blurred shadows retain contributing pixels outside the page and clip their
filtered result in native previews. Runtime dependencies are unchanged.

The qualified feature checks cover 27 block, table and page-edge scenarios,
including 16 complete clear-region comparisons at 300 DPI. Direct and compiled
PDFs, native previews and independent PDFium pixels match across the retained
Windows and Linux builds. The release pipeline runs the same installed-output
checks and retains their artifacts.

The unchanged 1,662-case CSS comparator reports 1,620 PASS, 24 FAIL and 18
REFERENCE-DISPUTED results. Its conclusion remains failure: a clipping case is
classified FAIL despite improved pixels, and rounded blur differs from Chrome
print output. These reviewed differences remain visible in the evidence; the
patch does not claim complete CSS or PDF standards conformance.

See the [2.5.28 runbook](docs/release/2.5.28-runbook.md) for the retained evidence
and publication checks. The experimental Bootstrap component alpha and the
Node/.NET engine versions are unchanged by this release.

---

# Fullbleed Release Notes - 2.5.27

Document CSS now reaches inline SVG shapes and text through the native cascade,
including ancestor selectors, stylesheet selection, layers, custom properties
and `currentColor`. SVG assets retain their own stylesheet context.

SVG group opacity composites overlapping shapes once. Paint alpha, inherited
fill/stroke opacity and group opacity remain separate; explicit root-opacity
inheritance, translucent `currentColor`, transparent masks and form reuse are
covered by installed-output fixtures. Masks keep the existing binary luminance
subset. No runtime dependency is added.

The feature checks cover 54 fixtures and 810 installed-output checks across
Windows and Linux builds. The complete 1,662-case CSS comparator retains the
baseline's 1,619 passes, 25 failures and 18 disputed references. One passing
fixture has a reviewed antialiasing change at a translucent rectangle's perimeter;
its slightly higher mismatch score is retained in the evidence.

See the [inline SVG styling guide](docs/embedded-stylesheets.md) and
[2.5.27 runbook](docs/release/2.5.27-runbook.md) for checks and limitations.

---

# Fullbleed Release Notes - 2.5.26

Render a complete HTML document with its embedded `<style>` sheets. Supported
static CSS now reaches direct rendering, compiled templates, previews, reporting
and batches without requiring callers to extract it into a separate argument.
Embedded `@page` rules also initialize streaming batches with the first record's
paper size.

CSS bundle assets, active embedded sheets and explicit CSS share the native
cascade. Collection preserves document order, raw HTML style text, print-media
conditions, stylesheet type, inert templates and static preferred sets. Each
source has a separate parsing boundary, and malformed sheets or unsupported
imports produce structured diagnostics. External stylesheets are still supplied
explicitly; the engine does not fetch them or execute JavaScript.

The installed runtime advertises `embedded_stylesheets`. Compiled templates
capture their stylesheet sources; changing those sources requires compilation
again. Inline SVG retains its existing separate stylesheet handling, documented
in the guide and tracked in issue #87. No required runtime dependency is added.

The release check exercises 195 installed-output cases, retaining PDFs, extracted
text, page boxes, native previews and independent PDFium images. The complete
1,662-case source corpus retains the same 25 failures and 18 disputed references
as the master baseline; this is regression evidence, not general CSS parity.

See the [complete HTML guide](docs/embedded-stylesheets.md) and
[2.5.26 runbook](docs/release/2.5.26-runbook.md).

---

# Fullbleed Release Notes - 2.5.25

Build complete PDF documents with React-inspired Python functions, props,
children and conditionals. Named text and document elements support rich
children. `StyleSheet` composes Python style mappings, while document artifacts
render PDFs and PNGs through an explicitly configured engine. The new rendering
path supplies overridable layout defaults for standalone rows, stacks and grids.

Nested lists and ordered generators become real children instead of printed
Python representations. Boolean children act as conditional placeholders; ARIA
and data boolean attributes retain explicit string values. Invalid HTML names,
cycles and content on void elements fail clearly. Inline style composition
preserves quoted values, data URLs and nested CSS blocks. Decorated functions
retain their signatures and documentation.

The component guide and designed invoice demonstrate the workflow without
authored HTML. Installed-package checks cover empty, paid, unpaid, escaped,
credit and long-table data, plus layout geometry, glyphs, previews and repeat
rendering. The explicit HTML/CSS API and Rust renderer remain unchanged. No
runtime dependency is added. Node, Rust and .NET component ports are not part of
this release.

See the [component guide](docs/component-authoring.md) and
[2.5.25 runbook](docs/release/2.5.25-runbook.md). Structural authoring validation
does not establish PDF accessibility conformance.

---

# Fullbleed Release Notes - 2.5.24

Scripts and style elements are now hidden by default in HTML output, including
between block elements. Previously, their source text could appear in a PDF
depending on the surrounding layout.

Text extraction for visual paragraphs, lists, preformatted text, and captions
now respects computed `display:none` on descendants. Authors can deliberately
display script or style source using CSS. `display:revert` restores the
element's user-agent display default, including through layers and important
declarations. Whitespace around visible source follows the same display rules.

The installed-output gate compares rendered PDFs, extracted text, native
previews, and independent PDFium pixels with visible-content controls through
direct, compiled, fixed-binding, and reflow rendering. No runtime dependency is
added. Fullbleed still does not execute JavaScript or apply embedded stylesheets.

See the [2.5.24 runbook](docs/release/2.5.24-runbook.md) for release evidence and
validation requirements. These checks do not establish complete browser parity
or PDF standards conformance.

---

# Fullbleed Release Notes - 2.5.23

HTML text using `text-decoration-thickness: from-font` now draws underlines,
overlines, and strike-throughs at the first available font's preferred
underline thickness. Previously, authored HTML used the automatic thickness
even when a registered font supplied a different value.

The font metric is scaled from its native design units to avoid intermediate
rounding. Fonts without a positive preferred thickness retain the automatic
fallback. Automatic-thickness and explicit-length behavior is unchanged. The
existing placement and minimum-width rules still apply to the selected stroke.

The installed-output gate covers Noto Sans, Inter, an unavailable first family,
small text, and a font without a preferred thickness. It checks direct,
compiled, fixed-binding, and reflow PDFs, independent PDF stroke widths,
native/finalized previews, explicit-length controls, and deterministic replay.
No runtime dependency is added.

This is a targeted HTML text-rendering correction. It does not establish
complete CSS parity or PDF standards conformance. See the
[2.5.23 runbook](docs/release/2.5.23-runbook.md).

---

# Fullbleed Release Notes - 2.5.22

Floated `::first-letter` initials in plain-text blocks now reserve their full
margin-box width for the lines beside them. Left and right drop caps keep
their padding, borders, backgrounds, and indentation, and sit in their own
line box instead of protruding above the paragraph's normal text baseline.

Words that cannot fit beside the initial move below it. Paragraph splitting
keeps the initial intact when the remaining page space is too short. Existing
inline first-letter and `initial-letter` behavior is preserved.

The installed-output gate covers 20 browser-checked cases across direct,
compiled, and reflow PDFs, with independent text/geometry and decoration
checks, native previews, and deterministic replay. The public 2.5.21 negative
control fails the 18 float cases while both controls pass. No runtime
dependency is added.

This fixes the existing horizontal plain-text block path; it does not expand
first-letter selection into arbitrary nested inline content or general float
layout. It does not establish complete CSS parity or PDF standards conformance.
See the [2.5.22 runbook](docs/release/2.5.22-runbook.md).

---

# Fullbleed Release Notes - 2.5.21

Border images using `space` now leave a region empty when no complete tile
fits. Previously, a partial tile could appear inside a narrow border or center
region. Complete tiles retain their existing spacing and centering.

The center fill now skips a zero-width top or left edge when choosing its
natural scale. It uses the opposite edge, or the unscaled source when neither
edge supplies a scale. This preserves the intended repeat pattern instead of
stretching it to the destination region.

Installed-output regression coverage includes 18 raster and SVG cases, with
native/finalized previews, independently rendered direct/compiled PDFs, and
deterministic rerenders. Browser expectations and public-release negative
controls are retained with the release evidence. No runtime dependency is added.

These are targeted rendering fixes. They do not establish general CSS parity
or PDF standards conformance. See the [2.5.21 runbook](docs/release/2.5.21-runbook.md).

---

# Fullbleed Release Notes - 2.5.20

Filtered and positive-z-index descendants now retain their ancestors' overflow
clips when painting is deferred. This prevents a tall filtered child from
escaping a rounded panel with `overflow: hidden`. Nested clips, one-axis
clipping, `overflow-clip-margin`, and transforms retain their geometry without
making overflow create a stacking context.

Reusable drawing resources remain defined before deferred paint commands.
This also fixes filtered positive-z-index children disappearing when their
form definition was scheduled after its first use.

The installed-output regression gate covers 16 browser-checked cases in native
previews, finalized PDFs, and independent PDFium rendering, including compiled
documents, repeated records, and fixed/reflow bindings. CI retains the 35 PDFs
and their checks. No browser or PDF-reader dependency is added to the engine.

These are targeted rendering fixes, not general CSS parity or PDF standards
conformance. See the [2.5.20 runbook](docs/release/2.5.20-runbook.md).

---

# Fullbleed Release Notes - 2.5.19

Native Rust applications can render the table, flex, grid, and nested-layout
regression cases on Windows' default 1 MiB main-thread stack in unoptimized
builds. Previously, even a small table could terminate the process with a
stack overflow.

Container state and computed styles now own their wide values on the heap.
This reduces large builder temporaries and values retained during recursive
DOM and table compilation. Public container methods, style calculations,
and clone behavior are preserved.

Windows/Linux CI runs a standalone main-thread consumer in addition to the
Rust test harness. It verifies nine document cases and retains 33 PDFs across
direct, compiled, repeated, and reflow output. Windows checks the executable's
actual stack reserve; the check does not use a larger worker or linker stack.
The probes include 16 nested blocks, three nested tables, and styled,
paginated containers. They do not promise arbitrary nesting depth.

No new dependency or rendering capability is introduced. Existing CSS corpus
failures and PDF conformance limitations remain. See the
[2.5.19 runbook](docs/release/2.5.19-runbook.md).

---

# Fullbleed Release Notes - 2.5.18

CSS counters created by a heading now remain in scope for its following
siblings. A chapter that resets its section counter produces `2.1` for the next
section, instead of continuing the previous chapter at `2.3`. Nested branches
retain their own counter scopes.

Table groups, rows, and cells apply counter mutations once, in source order.
Plain table cells preserve generated labels, and boxless table elements do not
mutate counters. The predefined footnote counter continues across paragraphs;
counter increments clamp at the supported integer bounds.

The installed regression check covers twelve fixtures and 48 emitted PDFs across
direct, compiled, repeated, and reflow rendering, including page breaks.
Windows/Linux CI and the release workflow retain PDFs and extracted labels.

The existing nested-counter corpus fixture has a separate visual mismatch;
this release does not claim to fix that fixture or implement complete CSS
counter support. Public APIs and dependencies are unchanged. See the
[2.5.18 runbook](docs/release/2.5.18-runbook.md).

---

# Fullbleed Release Notes - 2.5.17

The native Rust API reference now starts with a complete first-PDF program,
explicit font setup, and a guide to choosing buffers, writers, files, previews,
and compiled templates. Core methods document their return values and failure
behavior, including fixed-layout versus reflowing record bindings and the
distinction between layout previews and previews of a saved PDF.

The font example loads bytes into an asset bundle so missing files and invalid
font data surface as errors. File-based font registration helpers retain their
existing behavior of skipping unreadable or invalid files. Writer/file methods
return byte counts, and file methods can leave partial output on failure.

Cargo's documentation link now points to the hosted Rust API reference. CI builds
that reference with broken internal links treated as errors, alongside the Rust
suite's executable documentation examples. This release changes documentation,
CI checks and release metadata; it does not change rendering algorithms or add
new conformance claims. See the [2.5.17 runbook](docs/release/2.5.17-runbook.md).

---

# Fullbleed Release Notes - 2.5.16

With only a regular face registered, `font-synthesis: weight` now permits only
synthetic bold, and `font-synthesis: style` permits only synthetic italics or
oblique text. Previously both values could enable both approximations.

The four synthesis controls now inherit independently. The shorthand resets
omitted controls; longhands support `auto`/`none`, and the style longhand also
supports `oblique-only`. CSS-wide values, layer rollback, declaration order and
invalid-value rejection have focused regression coverage. HTML decoration
geometry and SVG text use the independent style control. SVG stylesheet
`!important` declarations take precedence over normal inline declarations.

Small-cap controls gate the existing HTML fallback; position controls gate the
default footnote-call superscript fallback. These controls do not add general
OpenType small-cap or positional substitution, or suppress real selected faces.
Rust `TextStyle.font_synthesis_weight` now controls weight alone; callers that
also want to disable slant synthesis can set `font_synthesis_style` to
`FontSynthesisStyle::None`.

The installed regression checks cover 130 render cases and 221 pages, with
deterministic replay, extracted text and independent PDFium comparisons. See
[font controls](docs/font-registration.md#controlling-synthetic-styles) and the
[2.5.16 runbook](docs/release/2.5.16-runbook.md).

Two existing fixed-template limitations remain documented: synthetic small
caps uppercases binding-slot names, and finalized native previews can omit
synthetic-bold strokes even when the emitted PDF displays them correctly in
PDFium. Use reflow for small-cap bindings and an independent PDF viewer for
fixed-slot synthetic bold. No complete CSS Fonts or new standards-conformance
claim is made.

---

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
