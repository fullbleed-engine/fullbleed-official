# Registering regular and italic fonts

Register the font files your document uses with `PdfEngine(font_files=[...])`,
`font_dirs=[...]`, or an `AssetBundle`. Fullbleed reads these assets directly;
it does not search system fonts.

When regular and italic faces share a family name, normal text selects the
regular face even if the italic file was registered first. The default family
face prefers upright fonts, then the CSS weight-400 fallback order: 400 through
500, lighter weights descending, and heavier weights ascending. Both legacy
and typographic family names are recognized.

For example, after registering `DMSerifDisplay-Regular.ttf` and
`DMSerifDisplay-Italic.ttf` from this repository's
[`design_showcase/fonts`](../examples/design_showcase/fonts):

```css
body { font-family: "DM Serif Display"; font-style: normal; }
em { font-style: italic; }
```

Use explicit `@font-face` mappings when a document needs to choose exact source
faces, including fonts whose variant names differ from the family name:

```css
@font-face {
  font-family: "Brand";
  src: local("DMSerifDisplay-Regular");
  font-style: normal;
  font-weight: 400;
}
@font-face {
  font-family: "Brand";
  src: local("DMSerifDisplay-Italic");
  font-style: italic;
  font-weight: 400;
}
body { font-family: "Brand"; }
```

Here, `local()` refers to a font already registered with the engine. PostScript
names, full face names, and supplied source aliases remain available for exact
selection. Duplicate explicit aliases and equally ranked family faces retain
the first registration. Directory entries are processed in sorted path order;
use an explicit file list when duplicate fonts need a particular priority.

Starting with 2.5.8, documents that accidentally used an italic or bold family
default can change appearance and line breaks when the regular face is also
registered. Review existing PDF baselines when upgrading. To intentionally use
italics, set `font-style: italic` or map the exact face with `@font-face`.

The installed-wheel [family smoke check](../tools/smoke_font_families.py)
compares embedded face names, extracted text, and native/PDFium previews against
explicit-face controls for file, directory, bundle, fixed-template, and reflow
rendering. This is a focused regression check, not a claim of complete CSS font
matching conformance.

## Controlling synthetic styles

When the selected registered face lacks a requested bold or slanted style,
`font-synthesis` controls which approximations Fullbleed may draw:

```css
/* Permit artificial italics, but keep the regular face's weight. */
.label { font-weight: 700; font-style: italic; font-synthesis: style; }

/* Keep bold synthesis while disabling artificial italics. */
.total { font-weight: 700; font-style: italic; font-synthesis: weight; }

/* Disable all synthetic variants, or override just one control. */
.exact { font-synthesis: none; }
.upright { font-synthesis-style: none; }
```

The shorthand independently sets `weight`, `style`, `small-caps`, and `position`;
omitted controls become `none`. The longhands `font-synthesis-weight`,
`font-synthesis-style`, `font-synthesis-small-caps`, and `font-synthesis-position`
accept `auto` or `none`. The style longhand also accepts `oblique-only`, which
permits a synthetic `font-style: oblique` but does not synthesize an italic
request. These properties inherit. `initial` restores all synthesis controls;
`auto` is a longhand value, not a valid shorthand value.

The controls do not suppress a real bold or italic face already selected by
font matching. SVG text inherits the document's weight/style controls and
supports overrides in its own inline and embedded CSS. Small-cap controls govern
HTML's existing synthetic small caps; position controls govern the default
footnote-call superscript fallback. They do not add general OpenType small-cap
or positional-substitution support. Synthetic small caps with fixed binding
slots remain unsupported; use compiled reflow for those templates.
The finalized-PDF preview reader can also omit the synthetic-bold stroke on
fixed binding slots; inspect the emitted PDF in an independent viewer when
using that combination.

The [synthesis smoke check](../tools/smoke_font_synthesis.py) registers a known
regular font and compares each combination with explicit style controls in
ordinary, fixed, and reflow output. It retains PDFs, extracted text, and native,
finalized, and PDFium previews; small-cap positive controls cover ordinary and
reflow output. CI runs this check on Windows and Linux. The comparisons verify
the named controls, not complete CSS Fonts conformance.
