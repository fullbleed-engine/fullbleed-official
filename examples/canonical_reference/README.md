# Fullbleed Canonical Reference

This example is the canonical scaffold-shaped reference for Fullbleed as a static PDF library. It is intentionally broader than a smoke test: it exercises component composition, manual page boundaries, engine-rendered footers, layered CSS, font registration, bundled SVG images, inline SVG, raster image data URIs, linked and standalone HTML artifacts, PDF rendering, PNG preview generation, page data export, component mount validation, and run reports.

Run it from the repository root:

```powershell
.venv\Scripts\python.exe examples\canonical_reference\report.py
```

Generated artifacts are written to `examples/canonical_reference/output/`:

- `canonical_reference.pdf`
- `canonical_reference_page*.png`
- `canonical_reference.html`
- `canonical_reference_standalone.html`
- `canonical_reference.css`
- `canonical_reference_page_data.json`
- `canonical_reference_component_mount_validation.json`
- `canonical_reference_css_layers.json`
- `canonical_reference_run_report.json`

Useful environment switches:

- `FULLBLEED_VALIDATE_STRICT=1` fails on CSS layer warnings and validation warnings.
- `FULLBLEED_IMAGE_DPI=144` controls preview PNG resolution.
- `FULLBLEED_OUTPUT_DIR=path` writes artifacts somewhere other than `output/`.
- `FULLBLEED_DEBUG=1` emits a render debug trace.
- `FULLBLEED_PERF=1` emits a performance trace.
- `FULLBLEED_JIT_MODE=plan` forwards a JIT mode to `PdfEngine`.

The rendered PDF includes a dedicated pagination page. It shows the authored `Page(...)` sequence, the `.ref-page` break rule, and the `footer_first` / `footer_each` / `footer_last` templates applied by `PdfEngine`.

The example reads `_css_working/css_parity_status.json` and `_css_working/css_fixture_report.json` when they exist, so the appendix can include the current local CSS coverage snapshot. It still renders without those reports.

To create this scaffold shape as a new project, use:

```powershell
fullbleed new local reference ./my-reference-doc
```
