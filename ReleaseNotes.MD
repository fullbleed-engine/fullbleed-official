# Fullbleed Release Notes - 2.4.0

Release date: September 30, 2026

## Print identity and PDF/X-4 requirements

- PDF/X-4 and PDF/VT-1 force PDF 1.6, require a nonempty document title and explicit UTC write date, and reject structurally invalid ICC output intents.
- One identity model supplies XMP document/instance identifiers, VersionID, RenditionClass, trapping status, title, and coherent XMP/Info dates. Content and timestamps determine identifiers without an implicit clock dependency.
- `document_timestamp`, CLI `--timestamp`, and `--timestamp-source SOURCE_DATE_EPOCH` support current or pinned job dates. The resolved date is retained for replay.
- `render` and `verify` parse print output for the internal writer contract, including metadata agreement, page boxes, output intent, and forbidden interactive/external constructs. This is distinct from independent ISO validation.

## PDF/VT-oriented composition

- Source inputs, compiled copies, and fixed/reflow binding rows map to Job → Record → Document parts with final page ranges and `RecordLevel 1`.
- `pdf_vt_job` accepts recipient IDs, grouped documents, and typed metadata. Buffered, streamed, and parallel batch paths preserve source boundaries and validate declared counts.
- Nodes retain private Fullbleed DPM. This vocabulary does not claim CIP4, JDF, or ISO 21812 semantics.
- XObjects receive conservative file-scoped reuse hints. Opaque images with explicit rendering intent can be marked encapsulated; forms and alpha images remain conservative. No RIP performance claim is made.
- Inspection exposes ordered parts, IDs, metadata, record/document counts, range integrity, and reuse hints. CLI discovery and MCP expose the same print job options.

## Earlier unreleased fixes included

- Preserve explicit table-header relationships, spans, reading order, and standard tagged-PDF table attributes through pagination and compiled VDP; retain the fixed-binding boundary for tagged profiles.
- Improve engine-owned chart typography, project-font use, painted text coordinates, text indexing, and table-cell bounds.
- Preserve source document metadata, keep figure titles out of accessibility facts, and encode Unicode Info titles consistently with XMP.
- Restrict authoring-preview assets to explicitly supplied bytes, resolve compliance notices from the owning installed distribution, and correct replaced SVG viewport sizing after CSS object fit.
- Expose engine-owned PDF profile requirements and ICC structural inspection through runtime discovery.
- Restore the required CIDSet on PDF/A-1 subset font descriptors, found by the expanded veraPDF release check.

## Compatibility and verification

- Print profiles now fail when title or timestamp inputs are omitted; existing callers must provide them. Ordinary PDF defaults remain unchanged.
- Core `fullbleed` retains no third-party Python runtime, browser, system-PDF, system-font, or AI-vendor dependency. External validators are release tooling only.
- The release matrix remains 15 `cp310-abi3` wheels for CPython 3.10–3.14, with Rust 1.85 as MSRV. The audit crate remains 0.1.3 and the MCP adapter package remains 0.1.0.
- Release workflows require veraPDF and PDF/X checks, a separate pypdf-based check of all five VT specimens, exact source-to-page assignments, replay determinism, and 21 deliberately damaged PDFs rejected for the expected reasons. A rewrite positive control prevents parser/rewrite failures from masquerading as successful negative tests.
- Dedicated external PDF/VT preflight remains available as an additional gate when configured. Its absence does not prevent shipping the tested writer features with qualified claims.

Full PDF/VT-1 conformance has not been independently established for this release.
The release reports the exact tests performed and does not claim certification
or exhaustive ISO coverage. See `docs/release/2.4.0-runbook.md` and `docs/pdf-vt.md`.
