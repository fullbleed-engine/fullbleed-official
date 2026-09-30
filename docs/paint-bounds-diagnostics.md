# Paint-bounds diagnostics

JIT placement bounds and render-time text traces are observations of the engine
display list. They must use the same coordinate conventions as PDF and raster
emission, not a second HTML/CSS layout interpretation.

Display geometry is ordinarily in top-down page points. Graphics transforms are
emitted into PDF's bottom-up user space: translation negates Y, rotation negates
its angle, general affine matrices negate B/C/F, and CSS transform origins use
`page_height - y`. Diagnostic consumers conjugate those matrices through the
page-space/PDF-space basis change. Scaling about the page bottom is not the same
as scaling top-down coordinates directly.

`DrawStringTransformed` is deliberately different from `DrawString`: it stores a
PDF-space baseline and a linear text matrix. Its observed bounds include that
matrix before returning to page space. Graphics-state restoration restores font
name and size as well as the transform. JIT bounds and Python render-time text
traces share the transform conversion.

Each `jit.docplan.pages[]` item includes its actual logical `page_size` in points.
The document-level size remains for backward compatibility. The CLI overflow
check uses per-page dimensions when present and the document size for older
streams. Invalid/nonfinite dimensions or placement rectangles cannot produce a
successful strict overflow check. The existing 0.01pt edge tolerance is unchanged.

Flowable bounds metadata supplements paint observations; it does not suppress
subsequent commands. Clip paths are consumed without being counted as painted
paths. This catches later off-page paint even when an earlier flowable had valid
layout metadata.

These are conservative diagnostic bounds, not exact ink coverage: curve control
hulls, font advance boxes, glyph-run estimates, layout boxes and form extents can
exceed visible ink; arbitrary clipping/effects are not an exact painted-area
oracle. The overflow check is not a PDF/UA, archival or print-conformance verdict.
Raster/PDF inspection and applicable independent profile validation remain
separate evidence.

Regression coverage includes the compiled 640×320 chart with 18pt project-local
Inter labels and a semantic table, actual native pixel comparisons for scale,
rotation, CSS origins and affine matrices, all four off-page edges, metadata
ordering, graphics-state restoration, transformed text and named-page dimensions.
No source CSS or verification threshold is changed to make these checks pass.
