# Native preview font substitutes

These compact, renamed OFL derivatives are compiled into Fullbleed. Unembedded
Standard 14 fonts can therefore be previewed without installed system fonts.
Registered and embedded font programs take precedence. The substitutes are
used only for rasterization; PDF font resources, text, and advances are unchanged.

The 12 Latin faces derive from Liberation 2.1.5. Symbol and Dingbats combine
outlines from the bundled Noto faces and Liberation. They cover every glyph in
the pinned AFMs' built-in encodings and 4,171 of the 4,172 named AFM glyphs. The
unencoded `Symbol` `/apple` vendor-private logo at U+F8FF is unsupported. These
are fixed substitutes, not exact reproductions of the original type designs.

The generator subsets and renames the fonts, decomposes outlines, omits hinting,
uses Adobe AFM advances, and maps legacy Adobe private-use characters to matching
Unicode outlines. The Latin spacing comma accent is positioned using its AFM
bearing. Legacy Symbol assembly pieces are fitted to their AFM glyph bounds so
radical bars and delimiter extenders retain their intended placement and extent.
The original PDF character mapping is never changed. Original notices
and OFL terms are in `LICENSE-*.txt`; Adobe metric and glyph-list notices are in
the repository's `THIRD_PARTY_LICENSES.md`.

To reproduce or check the files, install the development-only `fontTools==4.65.0`
and run `python tools/generate_preview_fonts.py` or append `--check`. The script
verifies all pinned inputs, downloads the hash-pinned Liberation archive into
`target/preview-font-sources` if needed, and writes deterministic timestamps.
No font generation tools or network access are needed by the runtime or build.
