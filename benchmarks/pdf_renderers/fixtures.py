"""Shared static document inputs and acceptance contract; no renderer branches."""
from __future__ import annotations

import base64
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FONT = ROOT / "fonts" / "Inter-Regular.ttf"
TOKEN = "RUN-000000"
NAMES = ("invoice", "ledger", "report")


class BodyText(HTMLParser):
    def __init__(self):
        super().__init__()
        self.segments = []

    def handle_data(self, text):
        if text.strip():
            self.segments.append(text.strip())


def stylesheet() -> str:
    face = base64.b64encode(FONT.read_bytes()).decode("ascii")
    return f"""@font-face {{ font-family: Inter; font-style: normal;
      font-weight: 400; src: url(data:font/ttf;base64,{face}) format('truetype'); }}
@page {{ size: A4; margin: 36pt; }}
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; padding: 0; }}
body {{ font-family: Inter; font-weight: 400; font-feature-settings: 'calt' 0; font-size: 9pt;
  line-height: 1.35; color: #172b3b; }}
h1, h2, h3, p {{ margin: 0 0 9pt; font-weight: 400; }}
h1 {{ font-size: 28pt; line-height: 1.15; }}
h2 {{ font-size: 17pt; line-height: 1.2; }}
h3 {{ font-size: 12pt; }}
.masthead {{ padding: 18pt; background: #123e4b; color: white;
  margin-bottom: 18pt; border-bottom: 5pt solid #dfb878; }}
.eyebrow {{ font-size: 8pt; letter-spacing: 1pt; margin-bottom: 9pt; }}
.meta {{ color: #4b6370; margin-bottom: 16pt; }}
.callout {{ background: #eaf2f1; border-left: 4pt solid #247b7a;
  padding: 14pt; margin: 14pt 0; }}
table {{ width: 100%; border-collapse: collapse; table-layout: fixed;
  margin: 12pt 0; }}
th, td {{ padding: 5pt 6pt; border-bottom: 0.5pt solid #bccbd0;
  text-align: left; vertical-align: top; font-weight: 400; }}
th {{ background: #eaf2f1; color: #123e4b; }}
thead {{ display: table-header-group; }}
tr {{ break-inside: avoid; }}
.right {{ text-align: right; }}
.total {{ text-align: right; font-size: 16pt; margin: 18pt 0; }}
.footnote {{ font-size: 8pt; color: #4b6370; }}
.section {{ break-before: page; }}
.section:first-child {{ break-before: auto; }}
.ledger td, .ledger th {{ padding: 4pt 6pt; }}
"""


def document(name: str) -> tuple[str, dict]:
    if name == "invoice":
        items = [("INV-01", "Document template design", "480.00"),
                 ("INV-02", "Data mapping workshop", "240.00"),
                 ("INV-03", "Print layout review", "180.00"),
                 ("INV-04", "Font asset preparation", "120.00"),
                 ("INV-05", "Release handover", "160.00"),
                 ("INV-06", "Support session", "80.00")]
        rows = "".join(f'<tr><td>{code}</td><td>{label}</td><td class="right">{amount}</td></tr>'
                       for code, label, amount in items)
        body = f"""<div class="masthead"><p class="eyebrow">NORTHSTAR / STUDIO</p>
<h1>Invoice</h1><p>{TOKEN}</p></div>
<p class="meta">Issued 04 October 2026 / Due 18 October 2026</p>
<h2>Prepared for Alder Works</h2><p>42 Harbour Lane, Portland, OR 97201</p>
<table><thead><tr><th style="width:18%">Item ID</th><th style="width:60%">Description</th>
<th class="right" style="width:22%">USD</th></tr></thead><tbody>{rows}</tbody></table>
<p class="total">Total due: USD 1,260.00</p>
<div class="callout"><h3>Delivery complete</h3><p>Six services supplied under project NS-204.</p></div>
<p class="footnote">Synthetic comparison fixture. No payment is due.</p>"""
        contract = {"min_pages": 1, "max_pages": 1,
                    "markers": [r[0] for r in items], "row_text": [list(r) for r in items],
                    "required": ["Invoice", "Alder Works", "Total due: USD 1,260.00", "NS-204"]}
    elif name == "ledger":
        items = [(f"ROW-{i:03d}", f"Service entry {i:03d}", f"{i * 3}.00") for i in range(1, 101)]
        rows = "".join(f'<tr><td>{code}</td><td>{label}</td><td class="right">{amount}</td></tr>'
                       for code, label, amount in items)
        body = f"""<h1>Service ledger</h1><p class="meta">October 2026 / {TOKEN}</p>
<table class="ledger"><thead><tr><th style="width:22%">Row ID</th>
<th style="width:55%">Description</th><th class="right" style="width:23%">USD</th></tr></thead>
<tbody>{rows}</tbody></table><p class="total">Ledger total: USD 15,150.00</p>
<p class="footnote">LEDGER-END / All 100 entries included.</p>"""
        contract = {"min_pages": 3, "max_pages": 4,
                    "markers": [r[0] for r in items], "row_text": [list(r) for r in items],
                    "required": ["Service ledger", "Ledger total: USD 15,150.00", "LEDGER-END"],
                    "repeated_heading": "Row ID"}
    elif name == "report":
        sections = []
        required = []
        rows = []
        for i, title in enumerate(("Delivery overview", "Quality review", "Next quarter"), 1):
            code = f"SECTION-{i:02d}"
            required.extend([title, f"REPORT-END-{i:02d}"])
            section_rows = [(f"METRIC-{i}{j}", label, value) for j, (label, value) in enumerate(
                (("Documents delivered", "1,248"), ("Templates maintained", "12"),
                 ("Review cycles completed", "36"), ("Open actions", "4")), 1)]
            rows.extend(section_rows)
            cells = "".join(f'<tr><td>{m}</td><td>{label}</td><td class="right">{value}</td></tr>'
                            for m, label, value in section_rows)
            sections.append(f"""<section class="section"><div class="masthead">
<p class="eyebrow">NORTHSTAR / OPERATIONS / {code}</p><h1>{title}</h1></div>
<p class="meta">Quarterly review / Section {i} of 3</p>
<h2>A repeatable document workflow</h2>
<p>The team prepares structured records, reviews the print layout, and retains the final documents.
Each template uses explicit fonts and a fixed paper size so that a reviewer can inspect the same inputs.</p>
<div class="callout"><h3>Review note {i}</h3><p>Check the longest records, page transitions,
and line-item totals before publishing a template update.</p></div>
<table><thead><tr><th style="width:24%">Metric ID</th><th style="width:54%">Measure</th>
<th class="right" style="width:22%">Value</th></tr></thead><tbody>{cells}</tbody></table>
<h2>Follow-up actions</h2><p>Keep the source data with the template revision. Record any layout
differences and repeat the output checks when dependencies change.</p>
<p class="footnote">REPORT-END-{i:02d}{(' / ' + TOKEN) if i == 1 else ''}</p></section>""")
        body = "".join(sections)
        contract = {"min_pages": 3, "max_pages": 3,
                    "markers": [f"SECTION-{i:02d}" for i in range(1, 4)] + [r[0] for r in rows],
                    "row_text": [list(r) for r in rows], "required": required,
                    "page_markers": [f"SECTION-{i:02d}" for i in range(1, 4)]}
    else:
        raise ValueError(name)
    html = f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{name.title()} comparison</title></head><body>{body}</body></html>'
    contract.update({"fixture": name, "token": TOKEN, "page_size_pt": [595.276, 841.89],
                     "font_name_contains": "Inter", "text_page_inset_pt": 30})
    contract["markers"] = sorted(contract["markers"], key=html.index)
    text = BodyText()
    text.feed(body)
    contract["text_segments"] = text.segments
    return html, contract


def write_inputs(directory: Path) -> None:
    import json
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "shared.css").write_text(stylesheet(), encoding="utf-8", newline="\n")
    for name in NAMES:
        html, contract = document(name)
        (directory / f"{name}.html").write_text(html, encoding="utf-8", newline="\n")
        (directory / f"{name}.json").write_text(json.dumps(contract, indent=2) + "\n", encoding="utf-8")
