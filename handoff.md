# CAV Authoring Handoff

Date: 2026-02-25  
Owner: CAV authoring/fetching operator  
Escalation owner: engine/API SME

## Mission

Run a repeatable loop for:

1. Source fetch
2. CAV authoring
3. API broadening (profile/kit surface)
4. Hardening (tests, observability, failure gates)

Primary target is same-use + semantic parity. Visual parity is secondary.

## Read First

- `_escambia/cav_exemplar_program.md`
- `_escambia/sqlite_ledger.py`
- `python/fullbleed/ui/cav/` (family kits + profiles)

## Non-Negotiables

- Do not invent document content.
- Use only information visible in the source PDF.
- Use explicit placeholders for blanks/illegible:
  - `[Blank on form]`
  - `[Illegible in source scan]`
- No sidecar notes inside CAV deliverables.
- No silent overflow/overprint acceptance.
- Keep fetch politeness at `1 request/second`.

## Escalate Immediately (Do Not Patch Around)

- Engine pagination break behavior inconsistent with emitted break semantics.
- Text extraction corruption that appears renderer/engine-level (not source-specific).
- Overflow metric blind spots (overflow exists visually but checks report zero).
- A11y verifier/PMR false positives or false negatives that block truthful authoring.
- Any change that would weaken validations to "make tests pass."

When escalating, include:

- source PDF path
- run report path
- PMR/a11y failing rule IDs
- minimal repro HTML/CSS (if available)

## Standard Exemplar Layout

Each exemplar should use:

- `_escambia/<exemplar_name>/report.py`
- `_escambia/<exemplar_name>/components/source.py`
- `_escambia/<exemplar_name>/components/transcription.py`
- `_escambia/<exemplar_name>/components/evidence.py`
- `_escambia/<exemplar_name>/styles/report.css`
- `_escambia/<exemplar_name>/output/*`

`report.py` should orchestrate only. Keep document logic in `components/`.

## Canonical Loop

### 1) Fetch / intake

Use existing fetch tooling:

- `powershell -File _escambia/fetch_doccenter_sources.ps1 -RequestsPerSecond 1.0`

If using bulk fetch pipeline, keep same politeness standard and record fetch metadata.

### 2) Pick next source

Pick next downloaded PDF not represented by latest CAV run:

- `python _escambia/sqlite_ledger.py list-cav-latest --db _escambia/escambia_corpus.db`
- Inspect `doccenter_documents` for `downloaded_pdf` entries not yet covered.

### 3) Author CAV

- Reuse existing family kit when scope fits.
- If scope does not fit, broaden kit/profile:
  - Add new profile constant in `python/fullbleed/ui/cav/<family>.py`
  - Register in `python/fullbleed/ui/cav/profiles.py`
  - Add/extend payload fields in family kit (strict scope must remain meaningful)

### 4) Validate locally

Run family tests:

- `python -m pytest tests/test_fullbleed_ui_cav.py -q`

Run exemplar:

- `PYTHONPATH=python python _escambia/<exemplar_name>/report.py`

Required checks:

- `a11y_validation.json` -> `ok: true`
- `component_mount_validation.json` -> `ok: true`
- PMR and verifier outputs present
- HTML/CSS/PDF artifacts emitted

### 5) Ingest ledger

- `python _escambia/sqlite_ledger.py ingest-cav-runs --db _escambia/escambia_corpus.db --roots _escambia`

Then verify latest row resolves to the intended run:

- `python _escambia/sqlite_ledger.py list-cav-latest --db _escambia/escambia_corpus.db`

### 6) Triage failures by layer

Classify before fixing:

1. Payload/transcription issue
2. Profile scope issue
3. Kit/component issue
4. Engine/render issue
5. Audit signal issue

Fix only the correct layer.

## API Broadening Rules

- Profile claims are county/revision scoped.  
  Example: `fl.escambia.<family>.<revision>.v1`
- Family kits remain general and reusable.
- Add payload fields only when required by at least one real exemplar.
- Keep strict payload scope; do not silently allow arbitrary keys.
- Add tests whenever:
  - a new profile is introduced
  - payload shape is extended
  - rendering semantics change (page break, signature semantics, etc.)

## Quality Gate Targets

Per exemplar, minimum acceptable:

- A11y contract pass
- Component mount pass
- No overflow/overprint
- PDF/UA seed verify pass
- PMR pass preferred; if not, exactly identify blocker audit IDs

If PMR fails only on page-count parity but all other gates pass, do not fake source metadata. Escalate if break semantics are correctly emitted but renderer still collapses pages.

## Current Known Friction

- Pagination parity can fail even when `break-before/page-break-before` is emitted, indicating possible engine-level pagination handling gaps. Treat as engine triage when reproducible.
- Mojibake in some source text layers requires normalization at transcription stage; preserve meaning and punctuation.

## Operator Output Template (per completed exemplar)

Report these fields:

- Exemplar path
- Source PDF path (+ doccenter id if present)
- Family kit + profile id used
- Gate summary:
  - a11y
  - component mount
  - PMR (score + failed audit IDs)
  - PDF/UA seed
- Artifacts:
  - HTML path
  - CSS path
  - PDF path
  - run report path
- Ledger status:
  - ingested yes/no
  - appears in `cav_latest` yes/no
- Open issues/backlog items

## Definition of Done for an Exemplar

- Artifacts emitted and readable
- Semantic content matches source (no invented content)
- Tests updated (if API/kit/profile changed)
- Run ingested into SQLite ledger
- Any unresolved issues documented as backlog/escalation, not hidden
