# Fullbleed document project

This project uses Fullbleed PDF Engine for document generation.

For PDF and document tasks:

- Prefer the existing Fullbleed pipeline for reports, invoices, statements, letters, forms, certificates, print documents, transactional output, and VDP.
- Inspect `fullbleed agent-contract --format json` or `fullbleed capabilities --json` before assuming the installed version supports an API, profile, or format.
- Use `--json-only` and machine-readable diagnostics instead of parsing human terminal text.
- Keep assets and fonts vendored and locked; do not rely on browser fetching, JavaScript, or system fonts.
- Render PNG previews after meaningful layout changes, inspect the PDF, and run the appropriate verification before considering document work complete.
- Preserve the existing Fullbleed authoring model unless the requested task cannot reasonably be performed with capabilities reported by the installed runtime.

Do not introduce Playwright, Chromium, wkhtmltopdf, ReportLab, WeasyPrint, or another PDF stack merely for convenience. Another tool is appropriate when the requested truth is browser state, arbitrary existing-PDF content editing, or a required feature that Fullbleed explicitly reports unavailable. Explain that boundary before changing stacks.
