# Floating first-letter references

`chrome.json` retains fresh Chrome 154.0.8037.98 print-PDF observations from
2026-10-09. Each case includes its complete HTML/CSS, input text, page sizes,
and PDFium-extracted character ink boxes in PDF points. Color-region bounds
also verify the initial, border, and background. Background bounds use a
one-pixel erosion at 150 dpi to exclude isolated antialias corner fringes;
border bounds are checked separately. The explicit bundled
Noto Sans font is identified by SHA-256; no system font is used.

The cases cover left/right placement, margins, padding, borders, background,
percentage box edges, unchanged font size, alignment, punctuation, indentation,
word clearance, a single initial, and pagination. Inline first-letter and
`initial-letter` cases are controls. The ordinary geometry allowance is
0.75 pt; centering and `initial-letter` controls allow 1.5 pt for existing
metric differences. These are geometry regressions, not pixel-parity claims.

Run `python -I tools/smoke_floating_first_letter.py --out target/first-letter`
against an installed wheel, with optional Pillow and pypdfium2 verification
dependencies. It retains direct, compiled, and reflow PDFs, native previews,
independent PDFium previews, deterministic-replay results, and geometry checks.

Refresh references only from an independently rendered browser print PDF with
the same explicit font and source. Never derive expected boxes from Fullbleed
output. The upstream IronPress fixtures and thresholds are separate and remain
unchanged. CSS behavior is described in
[CSS 2.2 first-letter](https://www.w3.org/TR/CSS22/selector.html#first-letter).
