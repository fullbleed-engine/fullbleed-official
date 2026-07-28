# Font License Audit

Audit date: 2026-07-28

Scope: `REMOTE_ASSETS` in `python/fullbleed_cli/assets.py`.

Method:
- Checked each font URL is reachable.
- Checked each license URL is reachable.
- Checked license text contains expected marker for declared license.
- Enforced allowlist for redistribution review: `OFL-1.1`, `Apache-2.0`, `UFL-1.0`, `MIT`.

Result:
- Total fonts: 39
- Passed: 39
- Failed: 0

| Font | Kind | Version | License | Font URL | License URL | Status |
| --- | --- | --- | --- | --- | --- | --- |
| `arvo` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/Arvo-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/OFL.txt | `PASS` |
| `crimson-text` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/CrimsonText-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/OFL.txt | `PASS` |
| `eb-garamond` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/EBGaramond%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/OFL.txt | `PASS` |
| `fira-code` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/FiraCode%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/OFL.txt | `PASS` |
| `inter` | `font` | `4.0` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/inter/OFL.txt | `PASS` |
| `jetbrains-mono` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/OFL.txt | `PASS` |
| `lato` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/lato/Lato-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/lato/OFL.txt | `PASS` |
| `libre-barcode-128` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128/LibreBarcode128-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128/OFL.txt | `PASS` |
| `libre-barcode-128-text` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128text/LibreBarcode128Text-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128text/OFL.txt | `PASS` |
| `libre-barcode-39` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39/LibreBarcode39-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39/OFL.txt | `PASS` |
| `libre-barcode-39-extended` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39extended/LibreBarcode39Extended-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39extended/OFL.txt | `PASS` |
| `libre-barcode-39-text` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39text/LibreBarcode39Text-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39text/OFL.txt | `PASS` |
| `libre-barcode-ean13-text` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcodeean13text/LibreBarcodeEAN13Text-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcodeean13text/OFL.txt | `PASS` |
| `libre-baskerville` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/LibreBaskerville%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/OFL.txt | `PASS` |
| `lora` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/lora/Lora%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/lora/OFL.txt | `PASS` |
| `montserrat` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/Montserrat%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/OFL.txt | `PASS` |
| `noto-color-emoji` | `font` | `2.047` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/NotoColorEmoji-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/OFL.txt | `PASS` |
| `noto-sans-arabic` | `font` | `2.010` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/OFL.txt | `PASS` |
| `noto-sans-hebrew` | `font` | `2.003` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/OFL.txt | `PASS` |
| `noto-sans-italic` | `font` | `2.014` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans-Italic%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt | `PASS` |
| `noto-sans-jp` | `font` | `2.004` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/OFL.txt | `PASS` |
| `noto-sans-kr` | `font` | `2.004` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/OFL.txt | `PASS` |
| `noto-sans-mono` | `font` | `2.014` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/NotoSansMono%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/OFL.txt | `PASS` |
| `noto-sans-regular` | `font` | `2.014` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt | `PASS` |
| `noto-sans-sc` | `font` | `2.004` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/OFL.txt | `PASS` |
| `noto-sans-thai` | `font` | `2.002` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/NotoSansThai%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/OFL.txt | `PASS` |
| `noto-serif-italic` | `font` | `2.014` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif-Italic%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt | `PASS` |
| `noto-serif-regular` | `font` | `2.014` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt | `PASS` |
| `nunito` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/Nunito%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/OFL.txt | `PASS` |
| `open-sans` | `font` | `wdth,wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OpenSans%5Bwdth%2Cwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OFL.txt | `PASS` |
| `oswald` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/Oswald%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/OFL.txt | `PASS` |
| `playfair-display` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/PlayfairDisplay%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/OFL.txt | `PASS` |
| `poppins` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/Poppins-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/OFL.txt | `PASS` |
| `pt-serif` | `font` | `regular` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/PT_Serif-Web-Regular.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/OFL.txt | `PASS` |
| `raleway` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/Raleway%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/OFL.txt | `PASS` |
| `roboto-mono` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/RobotoMono%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/OFL.txt | `PASS` |
| `rubik` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/Rubik%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/OFL.txt | `PASS` |
| `source-code-pro` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/SourceCodePro%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/OFL.txt | `PASS` |
| `work-sans` | `font` | `wght` | `OFL-1.1` | https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/WorkSans%5Bwght%5D.ttf | https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/OFL.txt | `PASS` |

No license or source integrity issues detected in this pass.
