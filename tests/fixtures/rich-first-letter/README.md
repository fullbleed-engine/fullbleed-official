# Styled first-letter print references

`chrome.json` records Chromium 153.0.8010.12 print output from a fresh headless
browser, captured on 2026-10-21. Each case includes its complete HTML and CSS,
the PDF hash, glyph bounds, and colored initial/border/background bounds.
Both renderers use the explicitly supplied `NotoSans-Regular.ttf` identified by
the recorded SHA-256. Browser defaults for emphasis are overridden explicitly.

Run against a built and installed wheel:

```console
python tools/smoke_rich_first_letter.py --out target/rich-first-letter-check
```

The gate covers transparent styled descendants, physical left/right floats,
inline font/color/background styling, percentage float edges, punctuation,
whitespace, long-word clearance, and page continuation. Narrow inline initials
must stay with their word, including across a text-node boundary. Leading breaks
and atomic inline boxes must prevent selecting a later letter. Direct, compiled,
reflow, native previews, and render-time text order and paragraph ownership are
checked; repeat PDF output must be deterministic.

The 1.5-point geometry tolerance follows the existing first-letter gate and
allows outline/raster rounding. Chromium's sheet rounding is normalized before
comparing top-based glyph bounds. This is not a general browser-parity or
accessibility claim. Generated-content initials, decorated inline ancestors,
vertical text, and the separate `initial-letter` sizing model remain outside
this gate.
