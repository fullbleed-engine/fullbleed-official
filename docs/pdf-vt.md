# PDF/VT composition and print identity

`pdfx4` and `pdfvt1` force PDF 1.6 and require a nonempty document title,
an explicit timestamp, a structurally valid embedded ICC output intent, and
embedded fonts for text. Read `fullbleed capabilities --json` and
`fullbleed.pdf_profile_catalog()` for the authoritative profile catalog.

These are emission and internal inspection contracts. Dedicated external
PDF/VT validation is still required before claiming ISO PDF/VT-1 conformance.
The engine does not implement PDF/VT-2, PDF/VT-2s, or PDF/VT-3.

## Identity and dates

Python accepts `document_timestamp="2026-09-30T12:34:56Z"`, `"current"`, or
`"source-date-epoch"`. The CLI accepts `--timestamp` with the first two forms
or `--timestamp-source SOURCE_DATE_EPOCH`. Fixed dates use Gregorian UTC;
`SOURCE_DATE_EPOCH` must contain nonnegative integer Unix seconds.

`current` and the environment value resolve once when the engine is created.
The resolved value is exposed as `engine.document_timestamp` and retained in
CLI manifests. Reuse that value for replay; create a new engine or explicitly
provide the new write date for a later production job. The runtime does not
silently replace an omitted print timestamp with the Unix epoch.

XMP CreateDate, ModifyDate, MetadataDate and PDF/VT modification date share
that value. Info dates, title, and trapping state agree with XMP. DocumentID
is a content-derived SHA-256 UUIDv8; InstanceID includes the write date. The
trailer contains deterministic 16-byte IDs. Metadata includes VersionID `1`,
RenditionClass `default`, and Trapped `False`. These identifiers describe a
freshly generated document; they are not an editing history or persistent
customer identifier.

## Records, documents, and final page ranges

```python
import fullbleed

job = {
    "id": "september-statements",
    "metadata": {"product": "statement", "copies": 1},
    "records": [
        {"id": "000001", "metadata": {"postal_code": "60601"},
         "documents": [{"id": "statement"}, {"id": "insert"}]},
        {"id": "000002", "documents": [{"id": "statement"}]},
    ],
}
engine = fullbleed.PdfEngine(
    pdf_profile="pdfvt1",
    document_title="September statements",
    document_timestamp="2026-09-30T12:34:56Z",
    output_intent_icc="assets/sRGB.icc",
    output_intent_identifier="sRGB IEC61966-2.1",
    font_files=["assets/Inter-Variable.ttf"],
    pdf_vt_job=job,
)
engine.render_pdf_batch_to_file(
    ["<p>Statement A</p>", "<p>Insert A</p>", "<p>Statement B</p>"],
    "body { font-family: Inter; }", "statements.pdf",
)
```

The tree is `Job → Record → Document → pages`, with
`NodeNameList [/Job /Record /Document]` and `RecordLevel 1` (zero-based).
Input documents consume the declared leaves in order. Each compiled copy or
binding row also consumes one document leaf. Page ranges are derived after
pagination, so a long reflow row remains one document and stays with its
recipient. Buffered, streamed, and parallel batch APIs retain these boundaries.
The declared leaf count must match the rendered input count.

Without a job description, the engine generates one record/document per input,
copy, or binding row. A job with an omitted or empty `records` list uses the
same automatic grouping while retaining the job ID and metadata. IDs must be
nonempty; record IDs are unique within a job, and document IDs within a record.
`pdf_vt_job` is rejected for other PDF profiles.

The CLI accepts `--pdf-vt-job job.json` or inline JSON. A single-document CLI
render consumes one leaf. MCP render and compile tools expose `pdf_vt_job`,
`document_timestamp`, `output_intent_icc_path`, and `font_paths`; paths are
confined to the server workspace. Rust exposes `PdfVtJob`, `PdfVtRecord`,
`PdfVtDocument`, `DpmMetadata`, and `DpmValue` through the builder.

## DPM and reuse hints

Every node carries `/DPM << /Fullbleed << /ID (...) /Metadata << ... >> >> >>`.
Metadata accepts UTF-8 strings, signed 64-bit integers, booleans, finite reals
within the PDF reader's single-precision range, lists, and nested dictionaries.
Use strings for exact decimal business values. Nulls and nonfinite numbers
are rejected. Limits are 16 levels and 10,000 values per node, 127 UTF-8 bytes
per key, 65,536 bytes per string, and 4,096 bytes per ID. Dictionary order is
canonical for deterministic output.

This is a private Fullbleed vocabulary. Postal codes, copies, media, finishing,
or binding fields carry caller-provided data; the engine does not interpret
them as finishing instructions or claim CIP4/JDF/ISO 21812 interoperability.
Agree those semantics with the print provider.

PDF/VT XObjects carry `GTS_Scope /File`. Only opaque images with an explicit
rendering intent receive `GTS_Encapsulated true`; forms and alpha images are
conservative `false`. Resources remain shared by the existing reuse machinery.
No cross-file reuse, DFE speedup, or RIP benchmark is implied by these hints.

## Inspection and independent validation

`fullbleed inspect pdf statements.pdf --json` reports the ordered parts, IDs,
typed DPM, record/document counts, final page ranges, graph validity, and reuse
hints. `render` and `verify` also parse print output and report
`pdf_profile_verification` with `scope="internal_writer_contract"` and
`independent_conformance=false`. Checks include identity consistency, ICC
structure, page boxes, and forbidden interactive/external constructs. This
does not prove arbitrary ICC colorimetry or every ISO requirement.

`tools/validate_pdf_profiles.py` retains PDFs, replay hashes, input hashes,
internal reports, PDF/X-4 reports, dedicated validator logs, and version output.
Dedicated validation covers the basic, multipage, grouped-record, fixed-binding,
and reflow specimens. Negative controls remove the DPart root reference,
change the PDF version, and break the VT/XMP date agreement.

Use `--pdfvt-cmd` (or `FULLBLEED_PDFVT_VALIDATOR_CMD`) with a `{pdf}` placeholder
and optional `{report}` placeholder. Templates use POSIX argument quoting on
all hosts and execute without a shell. A wrapper must select the actual
PDF/VT-1 validation profile and normalize success to exit 0 and nonconformance
to exit 1; crashes and configuration errors should use exit 2 or higher.
`--pdfvt-version-cmd` must retain the product version and selected profile.
Add `--require-dedicated-pdfvt --strict-external` for the release gate.

The release workflow requires repository variables `PDFVT_VALIDATOR_RUNNER`
(JSON label array, e.g. `["self-hosted","pdfvt"]`), `PDFVT_VALIDATOR_CMD`,
`PDFVT_VALIDATOR_VERSION_CMD`, and `PDFVT_OUTPUT_INTENT_ICC`. The runner must
already have a licensed or otherwise usable dedicated validator. Missing
configuration fails the release before publication. See the
[2.4.0 runbook](release/2.4.0-runbook.md) for evidence and publication requirements.
