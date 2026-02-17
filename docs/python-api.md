# Python API Reference

Primary import:

```python
import fullbleed
```

`fullbleed` re-exports the Rust extension API plus license helpers in `python/fullbleed/__init__.py`.

## Main classes and helpers

## `PdfEngine`

Main render entrypoint.

Common constructor options:

- page geometry: `page_width`, `page_height`, `margin`, `page_margins`
- rendering toggles: `reuse_xobjects`, `svg_form_xobjects`, `svg_raster_fallback`
- text controls: `unicode_support`, `shape_text`, `unicode_metrics`
- PDF config: `pdf_version`, `pdf_profile`, `color_space`, output intent fields
- document metadata: `document_lang`, `document_title`
- page template decorations: header/footer text and HTML variants
- watermark controls: `watermark_*` fields or `watermark=WatermarkSpec(...)`
- diagnostics: `jit_mode`, `debug/debug_out`, `perf/perf_out`
- paginated substitutions: `paginated_context={"key": "op"}`

Key methods:

- `register_bundle(bundle)`
- `render_pdf(html, css) -> bytes`
- `render_pdf_to_file(html, css, path) -> int`
- `render_pdf_with_page_data(html, css) -> (bytes, dict|None)`
- `render_pdf_with_glyph_report(html, css) -> (bytes, list[dict])`
- `render_image_pages(html, css, dpi=150) -> list[bytes]`
- `render_image_pages_to_dir(html, css, out_dir, dpi=150, stem=None) -> list[str]`
- batch APIs:
  - `render_pdf_batch`
  - `render_pdf_batch_to_file`
  - `render_pdf_batch_with_css`
  - `render_pdf_batch_with_css_to_file`
  - `render_pdf_batch_parallel`
  - `render_pdf_batch_to_file_parallel`
  - `render_pdf_batch_to_file_parallel_with_page_data`

## `AssetBundle`

Container for CSS/font/image/PDF/SVG assets.

- `add(asset)`
- `add_file(path, kind, name=None, trusted=False, remote=False)`
- `css() -> str`
- `assets_info() -> list[dict]`

## `AssetKind`

Class attributes:

- `AssetKind.Css`
- `AssetKind.Font`
- `AssetKind.Image`
- `AssetKind.Pdf`
- `AssetKind.Svg`
- `AssetKind.Other`

`Asset.info()` includes kind-specific metadata:
- `font`: primary font name (font assets)
- `pdf_version`, `page_count`, `encrypted` (PDF assets)

## `WatermarkSpec`

Constructor:

```python
fullbleed.WatermarkSpec(
    kind,
    value,
    layer="overlay",
    semantics=None,
    opacity=0.15,
    rotation_deg=0.0,
    font_name=None,
    font_size=None,
    color=None,
)
```

`kind` is one of: `text`, `html`, `image`.

## Helper functions

- `vendored_asset(source, kind, name=None, trusted=False, remote=False)`
- `fetch_asset(url) -> bytes`
- `concat_css(parts: list[str]) -> str`
- `finalize_stamp_pdf(template, overlay, out, page_map=None, dx=0.0, dy=0.0) -> dict`
- `finalize_compose_pdf(templates, plan, overlay, out) -> dict`

## License helper utilities

Process-local helpers exposed by `fullbleed`:

- `activate_commercial_license(...)`
- `clear_commercial_license()`
- `commercial_license_status()`

These exist for CLI/tooling interop and do not enforce licensing by themselves.

## Component-driven project pattern

For component-style reporting:

1. Keep components in `components/`
2. Keep CSS close to each component (component styles) and compose explicitly
3. Use a report entry module that builds HTML and CSS deterministically
4. Render through `PdfEngine` from that entrypoint

See scaffold template docs in `python/fullbleed_cli/scaffold_templates/init/SCAFFOLDING.md`.
