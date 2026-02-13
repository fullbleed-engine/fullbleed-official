# Scaffold + Engine Review (Human + AI Usability)

## Scope
This review reflects hands-on use of `fullbleed init` scaffold + engine during a real parity exercise against:
- `examples/fullbleed_dev_invoice/source.png`
- CSV-driven rendering via `examples/fullbleed_dev_invoice/data/invoice.csv`

Primary workflow tested:
1. Scaffold project structure
2. Build invoice with component-level Python + CSS
3. Render PDF + engine-native PNG preview
4. Use mount validation / CSS-layer checks to iterate

## Executive Summary
The current scaffold design pattern is strong: component-first, deterministic, and automation-friendly. The generated `report.py` gives a practical validation harness out of the box, which materially improves both human debugging and AI reliability.

The main friction is not architecture; it is engine-surface clarity around specific CSS/SVG behaviors. Most issues were resolvable, but required empirical tuning.

Overall assessment:
- Human usability: **8.0/10**
- AI usability: **8.8/10**
- Production readiness of scaffold pattern: **High**, with a few targeted docs/guardrail improvements.

## What Worked Well

### 1) Scaffold architecture is practical
- `components/` + `components/styles/` + `styles/` split works well.
- `components/primitives.py` is a good abstraction layer for reusable authoring patterns.
- Template files now come from `scaffold_templates/` instead of giant inline literals, which is easier to maintain.

### 2) Default `report.py` is unusually strong for a scaffold
The generated report includes:
- CSS layer ordering
- Mount smoke validation (`validate_component_mount`)
- CSS selector scoping checks
- Engine "known loss" declaration checks
- Optional JIT/perf/page-data/png outputs

This is a major win for observability and for AI agent workflows.

### 3) Deterministic render loop is fast to iterate
The command loop is clean and repeatable:
- run `python report.py`
- inspect `output/*.png`
- inspect mount validation JSON
- adjust component CSS

### 4) AI-operability is strong
The scaffolded contracts (`data-fb-role`, structured class names, diagnostics) made iterative layout repair significantly easier and safer than ad hoc HTML/CSS templating.

## Friction Encountered (Engine + Styling)

### 1) Some CSS is parsed but no-op
Observed and confirmed via mount validation signals:
- `font-variant-numeric` reported as parsed/no-effect

Impact:
- Numeric alignment improvements expected from tabular-nums are not available.
- Workaround used: monospace fallback for numeric columns.

### 2) SVG styling expectations vs engine behavior
Observed with inline icons:
- Class-based CSS styling for SVG primitives was less reliable than explicit SVG attributes.
- More robust result came from setting `stroke`, `stroke-width`, etc directly on SVG elements.

Impact:
- Authors may assume browser-like CSS-to-SVG styling semantics and get inconsistent output.

### 3) Background/page-fill behavior required explicit height semantics
Observed:
- A bottom strip remained unpainted until explicit page-height alignment was enforced (`document-root` height + inner section stretch).

Impact:
- `min-height` alone was not sufficient for predictable full-page background fill in this case.

### 4) Compact SVG path syntax was brittle
Observed:
- Some compact multi-command path strings were less reliable than explicit primitives (`line`, `rect`, `circle`) or clearer command separation.

Impact:
- Simple icons are safer when expressed with explicit shape elements.

### 5) CSS layering is powerful but needs explicit mental model
Observed during iterations:
- Global/base layers (`tokens`, `primitives`, `report`) can quietly influence component styles.
- The layer order in `report.py` solved this, but users need that order clear in docs.

Impact:
- Without clear precedence guidance, users may interpret normal cascade as "engine bug".

## Human Usability Review

Strengths:
- Scaffold is easy to start and modify.
- File layout is understandable.
- Validation artifacts reduce blind debugging.

Pain points:
- Some engine-specific CSS limitations only become obvious after render.
- SVG/CSS behavior differences from browsers require explicit guidance.

Human score rationale: **8.0/10**
- Strong architecture + defaults
- Deduction for CSS/SVG behavior surprises not yet fully codified in docs/checks

## AI Usability Review

Strengths:
- Predictable directory structure and component API
- Structured diagnostics and validation outputs
- Scoped selectors and no-effect checks are machine-tractable

Pain points:
- Engine-specific rendering semantics still require iterative discovery
- Some visual issues need empirical tuning (page fill, icon styling)

AI score rationale: **8.8/10**
- Better-than-average for agentic systems due to generated harness and diagnostics
- Still room for stronger engine-behavior hints and automatic fix suggestions

## Comparison vs Raw HTML/CSS Templating

Compared to raw template-only workflows:
- Better: maintainability, composability, testability, observability, AI friendliness
- Worse: slight upfront abstraction overhead and occasional need to adapt to engine-specific CSS support boundaries

Net: scaffolded component pattern is clearly the better default for teams and AI-assisted workflows.

## Recommended Improvements (Priority Ordered)

### P0 (high value, low risk)
1. Add a "supported vs parsed-no-effect" CSS appendix to scaffold docs, auto-linked from generated `COMPLIANCE.md` or `report.py` comments.
2. Add a scaffold note for SVG: prefer explicit presentation attributes for icon primitives in PDF workflows.
3. Add a short "CSS layer precedence" section to generated docs:
   - `tokens -> primitives -> component styles -> report`

### P1
1. Extend CSS/no-effect check output with actionable rewrite hints (for example: `font-variant-numeric -> use monospace numeric class`).
2. Add optional `FULLBLEED_VALIDATE_STRICT=1` default in CI recipe docs (not local default).
3. Add a scaffold utility snippet for page-fill-safe layouts (explicit page/root height contract).

### P2
1. Add an engine-native "style support report" command (or API) that can be run against authored CSS before render.
2. Add optional icon helper primitives that emit engine-safe SVG by default.

## Final Verdict
The scaffold pattern is on the right track and already materially better than most document-generation starters.

If the team tightens documentation around known CSS/SVG/page-fill semantics and keeps diagnostics first-class, this can be a very strong human + AI authoring surface for deterministic PDF production.
