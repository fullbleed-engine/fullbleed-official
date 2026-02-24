# Accessibility Examples Corpus (PDF/UA-Targeted Output Analysis)

This folder is a 3-track corpus for evaluating PDF output behavior through
`fullbleed.accessibility.AccessibilityEngine`.

The goal is to analyze PDF/UA-targeted tagged output quality and non-visual
observability artifacts, not just HTML authoring quality.

## Tracks

1. `cav_golden_marriage`
   - Reuses the real CAV canary (`keenan_coutney_marriage_cav`).
   - Expected: strong HTML/PMR results; PDF/UA seed gate passes; non-visual traces emitted.

2. `known_failure_legacy_bank_statement`
   - Uses a pre-accessibility legacy example (`examples/bank_statement`) through the
     `fullbleed.accessibility` runtime surface.
   - Expected: PMR/verifier warnings/failures (sanity check), but still produces a
     PDF/UA-targeted tagged PDF for trace analysis.

3. `wild_best_attempt_html`
   - Raw valid HTML/CSS (not authored with `fullbleed.ui.accessibility`) representing a
     plausible user "best attempt" at semantics.
   - Expected: mixed results; useful for comparing PDF output behavior against non-blessed input.

## Build

From repo root:

```powershell
$env:PYTHONPATH = "python"
.\.venv\Scripts\python.exe examples\_accessibility_examples\build_corpus.py
```

## Outputs

Artifacts are written under:

- `examples/_accessibility_examples/output/tracks/<track-name>/`

Corpus-level summary:

- `examples/_accessibility_examples/output/corpus_report.json`

Each wrapper-rendered track includes:

- HTML/CSS artifact pair
- tagged PDF (`pdf_ua_targeted`)
- engine a11y verifier + PMR reports
- PDF/UA seed verifier report
- post-render PDF traces (`lopdf`)
- render-time traces (`render_time_commands`)
- run report with cross-check summaries
