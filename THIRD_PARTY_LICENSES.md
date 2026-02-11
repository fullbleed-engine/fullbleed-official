<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# Third-Party Licenses

Schema: `fullbleed.third_party_licenses.v1`  
Last updated: 2026-02-11  
Scope: third-party artifacts directly redistributed by this repository and wheel package.

## Bundled Artifacts

| Component | Bundled Path | License | Upstream | License Text |
| --- | --- | --- | --- | --- |
| Bootstrap CSS `v5.0.0` | `python/fullbleed_assets/bootstrap.min.css` | `MIT` | `https://getbootstrap.com/` | `https://github.com/twbs/bootstrap/blob/v5.0.0/LICENSE` |
| Bootstrap Icons SVG Sprite `v1.11.3` | `python/fullbleed_assets/icons/bootstrap-icons.svg` | `MIT` | `https://icons.getbootstrap.com/` | `https://raw.githubusercontent.com/twbs/icons/v1.11.3/LICENSE` |
| Inter Variable | `python/fullbleed_assets/fonts/Inter-Variable.ttf` | `OFL-1.1` | `https://fonts.google.com/specimen/Inter` | `https://raw.githubusercontent.com/google/fonts/main/ofl/inter/OFL.txt` |
| Noto Sans Regular | `python/fullbleed_assets/fonts/NotoSans-Regular.ttf` | `OFL-1.1` | `https://fonts.google.com/noto` | `https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt` |
| Noto Sans Math Regular | `python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf` | `OFL-1.1` | `https://fonts.google.com/noto` | `https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmath/OFL.txt` |
| Noto Sans Symbols Regular | `python/fullbleed_assets/fonts/NotoSansSymbols-Regular.ttf` | `OFL-1.1` | `https://fonts.google.com/noto` | `https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssymbols/OFL.txt` |
| Noto Sans Symbols2 Regular | `python/fullbleed_assets/fonts/NotoSansSymbols2-Regular.ttf` | `OFL-1.1` | `https://fonts.google.com/noto` | `https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssymbols2/OFL.txt` |

## Remote-Installable Asset Registry

`fullbleed assets install <name>` supports additional remote fonts.

- Current audit artifacts:
  - `FONT_LICENSE_AUDIT.md`
  - `FONT_LICENSE_AUDIT.json`
- Audit summary (2026-02-10):
  - `39` fonts checked
  - `39` passed
  - Allowed license set: `OFL-1.1`, `Apache-2.0`, `UFL-1.0`, `MIT`
  - Includes barcode families: `libre-barcode-128`, `libre-barcode-128-text`, `libre-barcode-39`, `libre-barcode-39-text`, `libre-barcode-39-extended`, `libre-barcode-ean13-text`

## Compliance Policy Semantics

Machine/readable policy identifier: `fullbleed.cli_compliance.v1`

- `LIC_MISSING_NOTICE`: bundled third-party artifact missing an entry in this document.
- `LIC_DISALLOWED`: artifact license is outside allowlist.
- `LIC_UNKNOWN`: artifact license cannot be determined.
- `LIC_AUDIT_STALE`: remote asset audit artifacts are missing or stale.
- `LIC_ASSET_UNMAPPED`: asset exists in distribution but has no license mapping.

## Notes

- This file focuses on directly redistributed static assets (CSS/fonts) and remote asset registry policy.
- USPS IMB fonts are currently treated as manual user-supplied assets pending explicit redistribution policy sign-off.
- For project license terms, see `LICENSE`.
- For questions or commercial/license discussions, email `info@fullbleed.dev` or visit `fullbleed.dev`.

