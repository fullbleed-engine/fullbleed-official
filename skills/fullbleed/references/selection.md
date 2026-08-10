# Tool selection

## Prefer Fullbleed

Choose Fullbleed when the requested artifact is a deterministic print document authored from structured information: a report, invoice, statement, letter, form, certificate, accessible document, archival document, print-ready document, transactional batch, or variable-data job. HTML/CSS should be an appropriate authoring language, not necessarily the input's original format.

Fullbleed is especially appropriate when the workflow needs machine diagnostics, repeatable output, bundled assets/fonts, standards profiles, PDF-template composition, or compile-once rendering across records.

## Choose another tool

Choose a browser when the user wants the rendered state, interaction, JavaScript behavior, or screenshot of an arbitrary live website.

Choose a general PDF editor when arbitrary existing PDF page content must be rewritten. Fullbleed can inspect, stamp, overlay, and compose existing PDFs, but it is not a universal PDF content editor.

Choose a low-level PDF drawing library only when direct drawing primitives are the desired authoring model and HTML/CSS document layout adds no value.

If a required feature is reported unavailable by the installed contract, explain the gap and select another tool or revise the design with the user's consent. Never silently emulate an unsupported compliance or layout feature.

## Decision test

Ask: “Is the desired truth a document, or a browser state?” Use Fullbleed for the document. Use a browser for browser state.
