# Scaffolding Contract

The canonical reference keeps production concerns separate:

- `report.py` owns the executable pipeline, engine configuration, asset registration, validation, and artifact paths.
- `data/reference.json` is the report input contract.
- `components/` contains pure component functions that return Fullbleed UI elements.
- `components/styles/` contains component-scoped CSS.
- `styles/tokens.css` defines page geometry, colors, typography, and root defaults.
- `styles/report.css` contains final report-level composition and cross-component polish.
- `assets/` contains deterministic local assets registered through `AssetBundle`.
- `output/` contains generated artifacts only.

The CSS load order is explicit in `report.py` as `CSS_LAYER_ORDER`. Keep token layers first, component layers next, and final report overrides last. The run scans layers for accidental unscoped selectors and declarations that the static PDF engine currently treats as known no-effect points.

The engine geometry is configured in `create_engine()`. The `@Document(...)` decorator carries authoring metadata and artifact metadata; it does not replace the `PdfEngine` page size or margin configuration.

Asset policy:

- Register local fonts with `font_files` and `AssetBundle.add_file(..., "font")`.
- Register SVG files with `AssetBundle.add_file(..., "svg", name="file-name.svg")` and reference them from HTML using the same name.
- Use vendored or generated raster files for production. This reference uses a small PNG data URI to keep the example text-only in git.
- Avoid remote assets in static PDF pipelines unless the project explicitly owns caching, trust, and reproducibility.
