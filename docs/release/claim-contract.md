# Fullbleed 1.0 Claim Contract

This document defines public claim language for the 1.0 release line.

## Safe Claim

Fullbleed is a deterministic static PDF engine for transactional, report, VDP,
statement, invoice, menu, accessibility-audited, and chart-style documents. It
supports a broad, fixture-covered subset of HTML/CSS/SVG/image workflows, emits
reproducibility artifacts, and validates standards-oriented PDF profile output
through internal and external gates.

## Claims To Avoid

- Browser-compatible or browser-equivalent HTML/CSS rendering.
- Complete CSS3 support.
- Full SVG support.
- All image formats.
- Every generated PDF is accessible or PDF/UA conforming.
- Certified PDF/VT without a dedicated validator in the release gate.

## CSS

Claim:

- Broad static-document CSS subset with deterministic diagnostics and fixture
  evidence.

Boundary:

- The product target is static paged PDF output.
- JavaScript, interactive layout, browser print parity, complete grid/flex/table
  edge behavior, sticky positioning, 3D transforms, and full vertical text flow
  remain out of scope unless a specific fixture says otherwise.

## SVG

Claim:

- SVG document input, inline SVG in HTML, and SVG assets are supported for
  common static vector content.
- Distributed Python wheels enable `svg_raster`, so fallback-only SVG features
  rasterize through Fullbleed's dependency-free SVG compiler/Canvas pipeline.
- `fullbleed capabilities --json` reports the compiled SVG raster feature and a
  native/fallback/known-loss feature matrix.

Boundary:

- Native vector support is not full browser SVG.
- Filters, masks, patterns, and markers require raster fallback.
- SVG text/tspan runs, symbol/use viewports, and affine-transformed embedded
  images use the native vector display list. `foreignObject` content remains
  known-loss, matching the current static fallback renderer's behavior.

## Images

Claim:

- PNG and JPEG are the supported direct raster inputs.
- File paths, data URIs, registered bundle assets, `<img>`, CSS backgrounds,
  list-style images, and watermark images are supported within the documented
  static PDF scope.

Boundary:

- WebP, GIF, TIFF, AVIF, BMP, animated images, `<picture>`, `srcset`, `sizes`,
  density descriptors, and responsive image selection are not launch claims.
- Deterministic workflows should use local/vendored assets or explicit bundle
  assets. Remote assets must be explicitly allowed.

## PDF Profiles

Claim:

- PDF/A, PDF/UA, PDF/X-4, WTPDF, and PDF/VT-oriented output can be generated
  with inspectable and replayable evidence.
- The conformance harness regenerates profile specimens, inspects seed markers,
  checks determinism, and runs configured external validators.

Boundary:

- PDF/VT support is deterministic PDF/VT-1 seed/DPart support with PDF/X-4 base
  validation unless a dedicated PDF/VT validator is configured.
- `tagged` is a utility structural profile, not one of the 17 standard profile
  specimens in the conformance harness.

## Accessibility

Claim:

- Fullbleed provides PDF/UA-targeted workflows, seed checks, structure traces,
  and verifier/audit artifacts.

Boundary:

- Accessibility cannot be fully automated. Alt text quality, reading order,
  semantic table quality, language, contrast, and WCAG/508 applicability remain
  document-authoring and review responsibilities.
