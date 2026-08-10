# Compliance and delivery gates

## Interpret profiles correctly

A requested PDF profile configures the engine's emission contract. It does not prove that source semantics, fonts, color inputs, output intents, or the final artifact satisfy every external requirement.

Read the installed `pdf_profiles`, aliases, and `pdf_profiles_requiring_output_intent`. Supply an appropriate ICC output intent when the selected PDF/A or PDF/X profile requires one. Embed fonts when required by the standard and the document.

## Accessible output

For PDF/UA or tagged output:

1. Use semantic HTML and meaningful reading order.
2. Set document language and title.
3. Use an embeddable font with complete glyph coverage.
4. Inspect structure-tree, marked-content, language, metadata, and seed-blocker signals.
5. Run the applicable independent conformance checker when the delivery claim requires it.

Never describe a file as compliant solely because its metadata contains a standards identifier.

## Archival and print-ready output

For PDF/A and PDF/X, validate the canonical profile name, output-intent requirement, embedded fonts, metadata, color-space constraints, and final inspector signals. Retain the external validator report with the artifact when one is used.

## Reproducibility

Use `--repro-record` to record input and output fingerprints and `--repro-check` on a clean rerun. Preserve asset locks and deterministic hashes. Report reproducibility as verified only when the check actually succeeds in the delivery environment.

## Minimum delivery evidence

Keep the source/data inputs, Fullbleed version, relevant capability/manifest excerpt, render result, inspection result, verification result, standards-validator output when applicable, and artifact digest. These are evidence, not marketing claims.
