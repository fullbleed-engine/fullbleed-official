# Keenan/Coutney Marriage Record Accessibility CAV

Accessibility-first CAV (Corrective/Canonical Accessible Version) for the image-only source PDF:

- `examples/accessibility_test/keenan_coutney_marriage.pdf`

This project treats:

- HTML + CSS as the deliverable
- FullBleed as the preview + validation harness for iterative correctness

## Deliverables

- `output/keenan_coutney_marriage_cav.html` (emitted accessible HTML)
- `styles/report.css` (editable CSS deliverable)

Preview/harness artifacts:

- `output/keenan_coutney_marriage_cav.pdf`
- `output/keenan_coutney_marriage_cav_page*.png`
- `output/keenan_coutney_marriage_cav_a11y_validation.json`
- `output/keenan_coutney_marriage_cav_component_mount_validation.json`
- `output/keenan_coutney_marriage_cav_a11y_verify_engine.json` (engine-native verifier report; includes contrast seed from preview PNG when available)
- `output/keenan_coutney_marriage_cav_pmr_engine.json` (engine-native paged media rank report)
- `output/keenan_coutney_marriage_cav_source_analysis.json`
- `output/keenan_coutney_marriage_cav_transcription.json`
- `output/keenan_coutney_marriage_cav_parity_report.json`
- `output/keenan_coutney_marriage_cav_run_report.json`
- `output/keenan_coutney_marriage_cav_source_page1.png` (if PyMuPDF/`fitz` is available)

## Why This Exists

The source document is an image scan (no usable text layer). This CAV demonstrates:

- text-first semantic reconstruction with `fullbleed.ui.accessibility`
- explicit signature semantics (`wet_ink_scan`, `present`)
- review-queue preservation for low-confidence handwritten fields in sidecar JSON
- iterative render/validate loops while refining transcription and layout fidelity

## Run

From the repo root:

```bash
PYTHONPATH=python python examples/accessibility_test/keenan_coutney_marriage_cav/report.py
```

## Iteration Loop

1. Update transcription values / review notes in `report.py`
2. Adjust print layout in `styles/report.css`
3. Re-run `report.py`
4. Compare:
   - source scan preview (`..._source_page1.png`)
   - rendered preview PNG(s) (`..._page1.png`, etc.)
   - engine verifier/PMR outputs (`..._a11y_verify_engine.json`, `..._pmr_engine.json`)
5. Keep unresolved handwriting as explicit review items instead of forcing low-confidence text

## Current Known Uncertainties

- Notary/official handwritten signature name in fields 12 and 16 (appears to be "Ann Smith")
- `By D.C.` initials field (20c)
- Handwritten performer name/title text (field 23b) needs confirmation
- Witness names for certificate signatures (fields 24 and 25) are not confidently legible
