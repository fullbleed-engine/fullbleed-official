# Page-band authoring qualification

Local qualification completed on 2026-10-19 with the final built and installed
Windows x64 wheel. `PageBand`, `PageNumbers` and `PageBandError` are **unreleased
experimental source APIs**. The private candidate retains development version
`2.6.0a8`; it is not the published PyPI a8 wheel and must not replace that release.

Candidate wheel SHA-256:
`1e43d0369f70bdfe32c1fc5e1550c9ba4405343e36c225c27676cdd9467ba636`.
The [machine-readable receipt](component-page-bands-validation.json) records
runtime source hashes, fixture hashes, validator identities and report hashes.
The runtime source was unchanged between the wheel build and qualification.

The initial external check caught H2 as the first heading in the record smoke
fixtures. Those fixtures now explicitly author H1 and enable strict source
semantics. The affected matrix and external checks were rerun; the initial
failures remain in the evidence directory. No runtime change was needed.

## Results

| Check | Result |
| --- | --- |
| Native suite | 1,480 tests and 11 documentation tests passed |
| Installed-wheel Python checks | 216 focused tests and 970 full-suite tests passed |
| New page-band matrix | 33 cases passed across tagged, PDF/UA-1 and PDF/UA-2 output |
| Native page values | Correct final totals, first/blank-page policies and hidden numbers |
| Styled numbers | Class and inline styles agree; saved pixels agree with ordinary literal-text control components |
| Record output | Both compression modes; 1-, 3- and 1-page records agree with independently rendered controls and reset their page values |
| Existing output | 489 published-a8 fixture PDFs plus 30 prior paint-gate PDFs unchanged; 27 prior rejection payloads unchanged |
| Paired showcases | Published hashes preserved; Northstar remains byte-identical across authoring paths and Common Ground remains pixel-identical |
| External profile validation | All 32 new saved PDF/UA fixtures passed veraPDF; no stderr |
| Existing accessible guide | Both profile PDFs preserve published hashes and passed fresh veraPDF checks |
| New field-notes report | Three pages in each profile; semantic and independent PDF inspection passed; all pages visually reviewed |
| Browser export | Styled and unstyled HTML passed at 1440, 390 and 320 pixels; no horizontal overflow or page errors |
| Package integrity | Installed smoke, generated contract/stubs, typing, metadata, bundled Skill, workflow lint and three golden examples passed |

The 33-case matrix comprises 21 ordinary document cases, six combined-record
cases and six expected fit rejections. It saves 45 PDF files, including 18
independent record controls. External validation covers the 30 PDF/UA files
from that matrix plus the two PDF/UA field-notes reports. Tagged-only files are
not counted as externally validated PDF/UA output.

The reviewed report pages have separate body and margin content, readable
tables, and final `1 / 3`, `2 / 3`, `3 / 3` values. The report has identical
pixels across its three profiles. The browser export shows one header and
footer, honest `[page]` / `[pages]` placeholders, and no scripts. Its small
screen stylesheet adapts continuous layout; browser pagination is not implied.

## Reproduction and retained evidence

Build and install this source checkout before testing. Install the test-only
`pypdf`, `pypdfium2` and `Pillow` dependencies in the verification environment.
They are independent readers, not Fullbleed runtime dependencies.

```console
python -I tools/smoke_component_page_bands.py --out target/component-page-bands
python examples/component_page_bands/render.py --out target/page-band-report --pdf-profile pdfua2
python -I tools/smoke_component_typing.py --out target/component-typing.json
```

[`test_fullbleed_ui_page_bands.py`](../tests/test_fullbleed_ui_page_bands.py)
contains the API, CSS-ownership, styling and record-control regressions.
[`smoke_component_page_bands.py`](../tools/smoke_component_page_bands.py) saves
PDFs, exported source, independent previews and a structured verification report.
CI and release workflows run it against installed wheels and retain the output.
Remote CI results must be checked on the exact source revision separately.

Run veraPDF against every saved `pdfua1` and `pdfua2` PDF using its corresponding
`ua1` or `ua2` flavour. This qualification retained the complete JSON reports,
empty stderr files and saved inputs under
`target/growth/component-page-band-api-20261019/qualification-r2/page-band-validation/`.
The enclosing evidence directory also retains the private wheel, source snapshot,
logs, prior development failures, output comparisons and visual-review receipt.

## Scope and limits

These are bounded fixture results, not certification of every authored document
or human assistive-technology testing. Repeated bands are PDF artifacts; essential
information must also appear in body content. A pass does not establish PDF/VT
validation, which remains unconfigured, or fix the existing independent CSS
corpus baseline failures.

The [authoring contract](component-page-bands.md) specifies explicit embedded
fonts, slot ownership, conservative paint-fit rejection and HTML semantics.
Unsupported paint operations fail explicitly. Tight strokes can need extra
space. Labels use `prefix`, `separator` and `suffix`; generated pseudo-content
and placeholder child styling are not promised. Changed running bindings use
full reflow, and native streaming output remains non-atomic.
