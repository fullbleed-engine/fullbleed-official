# Static PDF Readiness Contract

This reference is considered correct when all of the following hold:

- The Python script exits successfully.
- The PDF exists and has non-zero size.
- Every preview PNG exists and has non-zero size.
- The component mount validation report has `"ok": true`.
- The CSS layer report has no unexpected unscoped component selectors.
- The output can be visually inspected as a multi-page static PDF with no obvious overlap, clipping, missing text, missing image assets, or blank pages.

Strict mode:

```powershell
$env:FULLBLEED_VALIDATE_STRICT = "1"
python report.py
```

Strict mode promotes CSS layer warnings and selected engine validation warnings to failures. It is intended for release checks after the example has been visually reviewed.

Known static-output boundaries:

- CSS animations, transitions, hover states, and script-driven interaction are not meaningful for deterministic static PDFs.
- This document targets the Fullbleed static PDF engine, not browser pixel parity.
- Production templates should vendor assets locally and should not depend on network-loaded CSS, fonts, or images.
