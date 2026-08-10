# Fullbleed workflows

Always inspect `fullbleed agent-contract --format json` first. The following patterns are navigation aids, not a substitute for the installed schemas.

## Ordinary document and preview

```text
fullbleed --json-only render --html document.html --css document.css --out output/document.pdf --emit-image output/preview --image-dpi 144
fullbleed --json-only inspect pdf output/document.pdf
fullbleed --json-only verify --html document.html --css document.css --fail-on overflow --fail-on missing-glyphs
```

Use project scaffolds when starting from a blank directory:

```text
fullbleed init . --json
fullbleed new local invoice . --json
```

## Existing PDF template

Inspect the source PDF before composition. Use `inspect templates`, a compose plan, and the `finalize compose` surface reported by the runtime. Preserve the source template as an input artifact and inspect the final PDF.

## Accessible document

Author semantic HTML, set a document title and language, bundle an embeddable font, select a reported `pdfua` or tagged profile, and verify the final structure/profile signals. Use the accessible scaffold when available. Read `compliance.md` before making claims.

## Fixed compiled VDP

Use a template containing exact `{{slot}}` text bindings. Compile once, then pass a columnar mapping whose keys exactly equal the compiled slots and whose string columns have equal non-zero length.

```python
import fullbleed

engine = fullbleed.PdfEngine()
compiled = engine.compile_pdf(template_html, css)
stats = compiled.stats()
compiled.render_pdf_bindings_to_file(bindings, "output/vdp.pdf")
```

Use this lane only when substituted text does not need to change geometry or pagination.

## Compiled reflow VDP

Use `render_pdf_reflow_bindings_to_file` when variable text or trusted structural slots change flow or page count. Select only a compression mode reported by `compiled_flow_compression_modes`.

```python
compiled.render_pdf_reflow_bindings_to_file(
    bindings,
    "output/reflow-vdp.pdf",
    compression="throughput",
)
```

Inspect `compiled.stats()` before rendering and validate record order and required markers after rendering.

## MCP

Start the first-party stdio adapter from the separate integration package:

```text
python -m pip install fullbleed-mcp
fullbleed-mcp --root .
```

Use `fullbleed_capabilities` or `fullbleed_agent_contract` before calling feature-specific tools. All user-supplied document paths are confined to the configured root.
