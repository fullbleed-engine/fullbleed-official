<!-- SPDX-License-Identifier: MIT -->
# Third-Party Licenses

Schema: `fullbleed.third_party_licenses.v1`  
Last updated: 2026-10-06
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
| Adobe Core 14 metrics | `src/base14_metrics_data.rs`; originals in `tools/data/base14/*.afm` | Adobe AFM permission below | Pinned Apache PDFBox mirror | `tools/data/base14/MustRead.html` |
| Adobe Glyph List | `src/base14_metrics_data.rs`; originals in `tools/data/base14/agl-*.txt` | BSD-3-Clause | `https://github.com/adobe-type-tools/agl-aglfn` | `tools/data/base14/agl-LICENSE.md` and copyright notices below |
| Fullbleed Preview Sans, Serif and Mono, derived from Liberation 2.1.5 | `src/preview_fonts/FullbleedPreview{Sans,Serif,Mono}-*.ttf`, compiled into the native preview renderer | OFL-1.1 | `https://github.com/liberationfonts/liberation-fonts/releases/tag/2.1.5` | `src/preview_fonts/LICENSE-Liberation.txt` |
| Fullbleed Preview Symbol and Dingbats, derived from Noto and Liberation | `src/preview_fonts/FullbleedPreview{Symbol,Dingbats}-Regular.ttf`, compiled into the native preview renderer | OFL-1.1 | Existing bundled Noto assets above and Liberation 2.1.5 | `src/preview_fonts/LICENSE-*.txt` |

The native renderer's Standard 14 preview substitutes are modified derivatives:
subsets with renamed families, decomposed unhinted outlines, Adobe AFM advances,
and preview-only aliases for legacy Adobe character mappings. Symbol assembly
pieces are fitted to their AFM glyph bounds. Original copyright
notices and complete license texts are retained in `src/preview_fonts/LICENSE-*.txt`
and in the Python wheel's license directory. `tools/generate_preview_fonts.py`
reproduces the fonts from the pinned inputs in `tools/data/preview_fonts/sources.json`;
output hashes and the single unsupported vendor-private logo are recorded in
`src/preview_fonts/sources.json`. These are substitute designs, not original
Standard 14 font programs. They do not change a PDF's font resources or text.

## Remote-Installable Asset Registry

`fullbleed assets install <name>` supports additional remote fonts.

- Current audit artifacts:
  - `FONT_LICENSE_AUDIT.md`
  - `FONT_LICENSE_AUDIT.json`
- Audit summary (2026-07-28):
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

- This file focuses on directly redistributed static assets (CSS/fonts/font metrics) and remote asset registry policy.
- USPS IMB fonts are currently treated as manual user-supplied assets pending explicit redistribution policy sign-off.
- For project license terms, see `LICENSE`.
- For licensing questions, email `info@fullbleed.dev` or visit `fullbleed.dev`.


## Adobe Core 14 metrics

The original AFM files and accompanying `MustRead.html` are retained unchanged in
`tools/data/base14`. Their source commits, SHA-256 hashes, and byte lengths are
recorded in `sources.json`. The generated Rust tables are modified derivatives:
Fullbleed extracts and reorganizes advances, glyph names, and character mappings
for native PDF preview. They contain no font programs or glyph outlines.

Original copyright and trademark notices:

Copyright (c) 1985, 1987, 1988, 1989, 1997 Adobe Systems Incorporated. All Rights Reserved.

Copyright (c) 1985, 1987, 1988, 1989, 1997 Adobe Systems Incorporated. All Rights Reserved.ITC Zapf Dingbats is a registered trademark of International Typeface Corporation.

Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.

Copyright (c) 1985, 1987, 1989, 1990, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.Times is a trademark of Linotype-Hell AG and/or its subsidiaries.

Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated.  All Rights Reserved.

Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated.  All Rights Reserved.Helvetica is a trademark of Linotype-Hell AG and/or its subsidiaries.

Copyright (c) 1985, 1987, 1989, 1990, 1997 Adobe Systems Incorporated. All rights reserved.

Copyright (c) 1989, 1990, 1991, 1992, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.

Copyright (c) 1989, 1990, 1991, 1993, 1997 Adobe Systems Incorporated.  All Rights Reserved.

Original redistribution permission (unchanged):

This file and the 14 PostScript(R) AFM files it accompanies may be used, copied, and distributed for any purpose and without charge, with or without modification, provided that all copyright notices are retained; that the AFM files are not distributed without this file; that all modifications to this file or any of the AFM files are prominently noted in the modified file(s); and that this paragraph is not modified. Adobe Systems has no responsibility or obligation to support the use of the AFM files.

## Adobe Glyph List

Copyright 2002-2019 Adobe (http://www.adobe.com/).

The generated character mappings derive from the Adobe Glyph List and its Zapf
Dingbats list. Original source files and license text are retained unchanged in
`tools/data/base14`. The redistribution terms below also apply to the derived
mappings in source and binary distributions.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

Redistributions of source code must retain the above copyright notice,
this list of conditions and the following disclaimer.

Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

Neither the name of Adobe nor the names of its contributors may be
used to endorse or promote products derived from this software without
specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
