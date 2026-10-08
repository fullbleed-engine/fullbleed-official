"""Inspect emitted counter labels across direct, fixed and reflow PDF paths."""
from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import version
import io
import json
from pathlib import Path
import re

import fullbleed
import fullbleed_assets
from pypdf import PdfReader


BASE_CSS = """@page { size: 400pt 500pt; margin: 24pt; }
* { margin: 0; padding: 0; }
body { font: 12pt Inter; line-height: 1.5; }
"""
CASES = [
    ("sibling-reset", "<div class='chapter'>Alpha</div><div class='section'>One</div><div class='section'>Two</div><div class='chapter'>Beta</div><div class='section'>Three</div>",
     "body { counter-reset: chapter; } .chapter { counter-increment: chapter; counter-reset: section; } .section { counter-increment: section; } .section::before { content: 'CNT' counter(chapter) '.' counter(section) 'END'; }",
     ["1.1", "1.2", "2.1"]),
    ("nested-branches", "<div class='scope'><div class='item'>Alpha<div class='scope'><div class='item'>Beta</div><div class='item'>Gamma</div></div></div><div class='item'>Delta</div></div><div class='scope'><div class='item'>Epsilon</div></div>",
     ".scope { counter-reset: n; } .item { counter-increment: n; } .item::before { content: 'CNT' counters(n, '.') 'END'; }",
     ["1", "1.1", "1.2", "2", "1"]),
    ("implicit-branches", "<section><div class='item'>Alpha</div><div class='item'>Beta</div></section><section><div class='item'>Gamma</div></section>",
     ".item { counter-increment: n; } .item::before { content: 'CNT' counter(n) 'END'; }",
     ["1", "2", "1"]),
    ("set-inherited-counter", "<section><div class='item'>Alpha</div><div class='set'>Beta</div></section><div class='item'>Gamma</div>",
     "body { counter-reset: n; } .set { counter-set: n 9; } .item { counter-increment: n; } .item::before { content: 'CNT' counter(n) 'END'; }",
     ["1", "10"]),
    ("list-sibling-reset", "<ol><li class='reset'>Alpha</li><li class='item'>Beta</li><li class='reset'>Gamma</li><li class='item'>Delta</li></ol>",
     ".reset { counter-reset: n 10; } .item { counter-increment: n; } .item::before { content: 'CNT' counter(n) 'END'; }",
     ["11", "11"]),
    ("table-cells", "<table><tbody><tr><td>Alpha</td><td>Beta</td></tr><tr><td>Gamma</td><td>Delta</td></tr></tbody></table>",
     "body { counter-reset: n; } td { counter-increment: n; } td::before { content: 'CNT' counter(n) 'END'; }",
     ["1", "2", "3", "4"]),
    ("table-groups", "<table><tbody><tr><td>Alpha</td><td>Beta</td></tr></tbody><tbody><tr><td>Gamma</td></tr></tbody></table>",
     "tbody { counter-reset: n 4; } td { counter-increment: n; } td::before { content: 'CNT' counter(n) 'END'; }",
     ["5", "6", "5"]),
    ("footnotes-across-paragraphs", "<p>Alpha<span class='fn'>First note</span></p><p>Beta<span class='fn'>Second note</span></p>",
     ".fn { float: footnote; } .fn::footnote-call { content: 'CNT' counter(footnote) 'END'; }",
     ["1", "2"]),
    ("integer-bounds", "<div class='high'>Alpha</div><div class='low'>Beta</div>",
     ".high { counter-reset: n 2147483647; counter-increment: n; } .low { counter-reset: n -2147483648; counter-increment: n -1; } div::before { content: 'CNT' counter(n) 'END'; }",
     ["2147483647", "-2147483648"]),
    ("paginated-sections", "<div class='chapter'>Alpha</div><div class='section'>One</div><div class='chapter next'>Beta</div><div class='section'>Two</div>",
     "body { counter-reset: chapter; } .chapter { counter-increment: chapter; counter-reset: section; } .next { break-before: page; } .section { counter-increment: section; } .section::before { content: 'CNT' counter(chapter) '.' counter(section) 'END'; }",
     ["1.1", "2.1"]),
    ("boxless-table-cell", "<table><tr><td style='display:contents;counter-increment:n 5'><span>Alpha</span></td></tr></table>",
     "body { counter-reset: n; } span::before { content: 'CNT' counter(n) 'END'; }",
     ["0"]),
    ("boxless-table-group", "<table><tbody style='display:contents;counter-reset:n 8'><tr><td><span>Alpha</span></td></tr></tbody></table>",
     "body { counter-reset: n; } td { counter-increment: n; } span::before { content: 'CNT' counter(n) 'END'; }",
     ["1"]),
]


def check(out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=False)
    engine = fullbleed.PdfEngine(font_files=[str(fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"))])
    records = []
    for name, body, rules, expected in CASES:
        html = "<html><body><p>Record {{name}}</p>" + body + "</body></html>"
        css = BASE_CSS + rules
        (out / (name + '.html')).write_text(html, encoding="utf-8")
        (out / (name + '.css')).write_text(css, encoding="utf-8")
        literal = html.replace("{{name}}", "Ada")
        direct = bytes(engine.render_pdf(literal, css))
        assert direct == bytes(engine.render_pdf(literal, css)), name
        fixed = engine.compile_pdf(literal, css)
        reflow = engine.compile_pdf(html, css)
        outputs = [
            ("direct", direct, 1),
            ("compiled", bytes(fixed.render_pdf()), 1),
            ("repeat", bytes(fixed.render_pdf_batch(2)), 2),
            ("reflow", bytes(reflow.render_pdf_reflow_bindings({"name": ["Ada", "Bea"]})), 2),
        ]
        for mode, data, repeats in outputs:
            reader = PdfReader(io.BytesIO(data))
            if name == "paginated-sections":
                assert len(reader.pages) == 2 * repeats, (name, mode, len(reader.pages))
            text = "\n".join(page.extract_text() for page in reader.pages)
            labels = re.findall(r"CNT([\d.\-]+)END", re.sub(r"\s+", "", text))
            assert labels == expected * repeats, (name, mode, labels, expected, text)
            file = f"{name}-{mode}.pdf"
            (out / file).write_bytes(data)
            records.append(dict(case=name, mode=mode, file=file, pages=len(reader.pages), labels=labels,
                                sha256=hashlib.sha256(data).hexdigest()))
    report = dict(ok=True, version=version("fullbleed"), cases=len(CASES), artifacts=records,
                  scope="Final PDF text and deterministic replay; no claim of complete CSS counter support or browser raster parity.")
    (out / 'verification.json').write_text(json.dumps(report, indent=2)+'\n', encoding="utf-8")
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.out)
    print(json.dumps(dict(ok=result['ok'], version=result['version'], cases=result['cases'],
                          pdfs=len(result['artifacts']))))
