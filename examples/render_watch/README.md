# Edit a document with automatic rebuilds

`--watch` requires **Fullbleed 2.5.0 or newer**. Check with
`python -m fullbleed --version`. To build a source checkout, use the
[contributor setup](../../CONTRIBUTING.md#set-up-a-source-checkout).

From this directory, run:

```sh
python -m fullbleed render --html invoice.html --css styles.css --asset "@noto-sans" --out invoice.pdf --emit-image preview --watch
```

Fullbleed renders once, then rebuilds after you save either source file. The font
is bundled in the wheel. Open `invoice.pdf` or the PNG in `preview/`; an image
viewer that supports file reloads is useful while editing. The command itself
does not open or refresh a viewer.

Try changing the `h1` color in `styles.css`, or adding a table row in
`invoice.html`. Amounts in this fictional sample are static; update the total
when changing line items. Fullbleed renders your content rather than calculating
invoice arithmetic.

Press **Ctrl-C** to stop. Omit `--watch` to render once. See the
[watch guide](../../docs/render-watch.md) for JSON output, extra dependencies,
debouncing, and error recovery.
