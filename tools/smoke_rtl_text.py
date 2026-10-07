"""Verify Arabic PDF text and styled RTL placement using an installed wheel."""
from hashlib import sha256
from importlib.metadata import version
from importlib.resources import files
from pathlib import Path
import argparse
import json
import unicodedata

import fullbleed
import pypdfium2 as pdfium
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
FONT_ROOT = ROOT / "tests/fixtures/rtl"


def check(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    checks = []
    def expect(name, result):
        checks.append({"name": name, "passed": bool(result)})

    provenance = json.loads((FONT_ROOT / "font-source.json").read_text(encoding="utf-8"))
    for asset in provenance["files"]:
        actual = sha256((FONT_ROOT / asset["file"]).read_bytes()).hexdigest()
        if actual != asset["sha256"]:
            raise RuntimeError("Regression font or license changed: " + asset["file"])

    inter = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    engine = fullbleed.PdfEngine(
        font_files=[str(FONT_ROOT / "NotoSansArabic-Regular.ttf"), str(inter)],
        document_lang="ar", document_title="Arabic text regression",
    )
    cases = {
        "marks": '<p>مرحباً بالعالم</p><p>يُرجى استخدام رقم الفاتورة</p>',
        "spans": '<p>رقم الفاتورة <span class="latin isolate">INV-2048</span> مستحق في <span class="latin isolate">2026-10-21</span>.</p>',
        "latin-siblings": '<p>قبل <span class="latin">Acme</span> <span class="latin">Studio</span> بعد</p>',
        "normal-date": '<p>رقم الفاتورة <span class="latin">INV-2048</span> مستحق في <span class="latin">2026-10-21</span>.</p>',
        "html-dir": '<p dir="rtl">رقم الفاتورة <bdi dir="ltr" class="latin">INV-2048</bdi> مستحق في <bdi dir="ltr" class="latin">2026-10-21</bdi>.</p>',
        "nested-isolate": '<p>رقم الفاتورة <bdi dir="ltr" class="latin"><span>INV-</span><span>2048</span></bdi> مستحق في <bdi dir="ltr" class="latin"><span>2026-</span><span>10-21</span></bdi>.</p>',
        "auto-isolate": '<p>رقم الفاتورة <bdi class="latin">INV-2048</bdi> مستحق في <bdi class="latin">2026-10-21</bdi>.</p>',
        "wrapping": '<p class="narrow">هذه فاتورة لتصميم الهوية البصرية <span class="latin isolate">INV-2048</span> وإعداد قوالب المستندات. يُرجى استخدام الرقم عند التواصل معنا <bdi dir="ltr" class="latin">2026-10-21</bdi>.</p>',
    }
    css = '''@page { size: A4; margin: 20mm; }
    body { font-family: "Noto Sans Arabic", Inter; font-size: 18pt; line-height: 1.8; }
    p { direction: rtl; text-align: right; }
    .latin { font-family: Inter; direction: ltr; }
    .isolate { unicode-bidi: isolate; }
    .narrow { width: 230pt; }
    '''
    results = {}
    for name, body in cases.items():
        case = out / name
        case.mkdir(exist_ok=True)
        html = '<html lang="ar"><body>' + body + '</body></html>'
        case_css = css.replace('p { direction: rtl; text-align: right; }', 'p { text-align: right; }') if name == "html-dir" else css
        pdf, missing = engine.render_pdf_with_glyph_report(html, case_css)
        (case / "document.html").write_text(html, encoding="utf-8")
        (case / "document.css").write_text(case_css, encoding="utf-8")
        path = case / "document.pdf"
        path.write_bytes(pdf)
        engine.render_finalized_pdf_image_pages_to_dir(str(path), str(case / "native"), 110, "page")
        reader = PdfReader(path)
        text = '\n'.join(page.extract_text() for page in reader.pages)
        (case / "pypdf.txt").write_text(text, encoding="utf-8")
        doc = pdfium.PdfDocument(pdf)
        page = doc[0]
        textpage = page.get_textpage()
        other = textpage.get_text_range()
        (case / "pdfium.txt").write_text(other, encoding="utf-8")
        page.render(scale=110 / 72).to_pil().save(case / "pdfium.png")
        expect(name + ": one page and no missing glyphs", len(reader.pages) == 1 and not missing)
        expect(name + ": no injected direction controls", not any(unicodedata.category(ch) == "Cf" for ch in text + other))

        def word_box(word):
            search = textpage.search(word)
            found = search.get_next()
            search.close()
            if found is None:
                return None
            first, count = found
            boxes = [textpage.get_charbox(index) for index in range(first, first + count)]
            return [min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes)]

        boxes = {word: word_box(word) for word in ["INV", "2048", "2026-10-21", "Acme", "Studio", "2026", "21"]}
        if name == "marks":
            for phrase in ["مرحباً", "يُرجى"]:
                expect("marked word appears once in pypdf: " + phrase, text.count(phrase) == 1)
                expect("marked word appears once in PDFium: " + phrase, other.count(phrase) == 1)
            expect("spaces remain ordinary spaces", "مرحباً بالعالم" in text)
        elif name == "latin-siblings":
            first, second = boxes["Acme"], boxes["Studio"]
            expect("LTR words retain order inside the RTL paragraph", first is not None and second is not None and first[2] < second[0])
        elif name == "wrapping":
            invoice, date = boxes["INV"], boxes["2026-10-21"]
            expect("wrapped fields retain their complete text", "INV-2048" in text and "2026-10-21" in text)
            expect("RTL paragraph wraps fields onto later lines", invoice is not None and date is not None and date[3] < invoice[1])
            # Every painted character must remain within the 230pt paragraph.
            bounds = [textpage.get_charbox(index) for index in range(textpage.count_chars()) if not textpage.get_text_range(index, 1).isspace()]
            left = 20 * 72 / 25.4
            expect("wrapped text stays inside its authored width", all(left - 1 <= b[0] and b[2] <= left + 231 for b in bounds))
        else:
            invoice, date = boxes["INV"], boxes["2026-10-21"]
            if name != "normal-date":
                # PDFium's line-level bidi heuristic can extract INV-2048 as
                # -2048INV, including in the browser reference. Check its glyph
                # positions separately and assert the intact source in pypdf.
                expect(name + ": source date and identifier survive extraction", "INV-2048" in text and "2026-10-21" in text)
                expect(name + ": identifier glyphs keep their LTR order", invoice is not None and boxes["2048"] is not None and invoice[2] < boxes["2048"][0])
                expect(name + ": date sits to the left of the identifier", invoice is not None and date is not None and date[2] < invoice[0])
            else:
                year, day = boxes["2026"], boxes["21"]
                expect("normal inline direction uses the paragraph's bidi context", year is not None and day is not None and day[2] < year[0])
        results[name] = {"pdf_sha256": sha256(pdf).hexdigest(), "bytes": len(pdf), "boxes": boxes, "missing": missing}
        textpage.close()
        page.close()
        doc.close()

    report = {"ok": all(item["passed"] for item in checks), "engine": version("fullbleed"),
              "engine_module": fullbleed.__file__,
              "pypdf": version("pypdf"), "pdfium": version("pypdfium2"), "checks": checks, "cases": results}
    (out / "verification.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    report = check(args.out)
    print(json.dumps({"ok": report["ok"], "checks": len(report["checks"]), "failures": [item["name"] for item in report["checks"] if not item["passed"]]}))
    raise SystemExit(0 if report["ok"] else 1)
