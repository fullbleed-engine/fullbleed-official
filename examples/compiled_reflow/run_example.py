from __future__ import annotations

import argparse
import html
import json
from pathlib import Path

import fullbleed


ROOT = Path(__file__).resolve().parent
OUTPUT = ROOT / "output"

TEMPLATE = """<!doctype html>
<html>
  <body>
    <main>
      <h1>{{document_title}}</h1>
      <p>Customer: {{customer}}</p>
      <p class="summary">{{summary}}</p>
      <h2>Account activity</h2>
      <table>
        <thead><tr><th>Date</th><th>Description</th><th>Amount</th></tr></thead>
        <tbody data-fb-bind-html="rows"></tbody>
      </table>
      <p class="end">END {{record_id}}</p>
    </main>
  </body>
</html>
"""

CSS = """
@page {
  size: letter;
  margin: 0.7in;
  @top-right { content: string(document-title); }
  @bottom-center { content: "Page " counter(page) " of " counter(pages); }
}
body { margin: 0; font: 10pt/1.3 Helvetica, sans-serif; }
h1 { margin: 0 0 10pt; string-set: document-title content(text); }
h2 { margin: 14pt 0 6pt; break-after: avoid; }
.summary { break-inside: avoid; padding: 6pt; border: 0.5pt solid #789; }
table { width: 100%; border-collapse: collapse; }
thead { display: table-header-group; background: #e6eef5; }
tr { break-inside: avoid; }
th, td { border: 0.5pt solid #9aa6b2; padding: 4pt; text-align: left; }
th:last-child, td:last-child { text-align: right; }
.end { margin-top: 10pt; }
"""


def render_rows(record_id: str, count: int) -> str:
    """Build trusted structure while escaping every externally sourced field."""
    rows: list[str] = []
    for index in range(count):
        date = f"2026-08-{index % 28 + 1:02d}"
        description = f"{record_id} item <Priority> {index:03d} " + "detail " * (
            index % 4
        )
        amount = f"${index * 7 + 10:,.2f}"
        rows.append(
            "<tr>"
            f"<td>{html.escape(date)}</td>"
            f"<td>{html.escape(description)}</td>"
            f"<td>{html.escape(amount)}</td>"
            "</tr>"
        )
    return "".join(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--compression",
        choices=("throughput", "compact"),
        default="throughput",
    )
    args = parser.parse_args()

    records = [
        ("REC-SHORT", "Avery <North>", 8),
        ("REC-MEDIUM", "Morgan & South", 38),
        ("REC-LONG", "Riley West", 92),
    ]
    bindings = {
        "record_id": [record_id for record_id, _customer, _count in records],
        "document_title": [
            f"Statement {record_id}" for record_id, _customer, _count in records
        ],
        "customer": [customer for _record_id, customer, _count in records],
        "summary": [
            f"Variable summary for {record_id}. " + "Review details. " * (index + 1)
            for index, (record_id, _customer, _count) in enumerate(records)
        ],
        "rows": [
            render_rows(record_id, count) for record_id, _customer, count in records
        ],
    }

    compression = {
        "throughput": fullbleed.CompiledFlowCompression.Throughput,
        "compact": fullbleed.CompiledFlowCompression.Compact,
    }[args.compression]
    engine = fullbleed.PdfEngine()
    compiled = engine.compile_pdf(TEMPLATE, CSS)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    pdf_path = OUTPUT / f"compiled-reflow-{args.compression}.pdf"
    hash_path = OUTPUT / f"compiled-reflow-{args.compression}.sha256"
    written = compiled.render_pdf_reflow_bindings_to_file(
        bindings,
        str(pdf_path),
        str(hash_path),
        compression=compression,
    )

    extracted = fullbleed.extract_pdf_page_texts(str(pdf_path))
    if not extracted["ok"]:
        raise RuntimeError(extracted)
    text = "\n".join(page["text"] or "" for page in extracted["pages"])
    missing = [
        record_id
        for record_id, _customer, _count in records
        if f"END {record_id}" not in text
    ]
    if missing:
        raise RuntimeError(f"missing record markers: {missing}")

    print(
        json.dumps(
            {
                "ok": True,
                "compression": args.compression,
                "records": len(records),
                "pages": len(extracted["pages"]),
                "bytes": written,
                "pdf": str(pdf_path),
                "sha256": hash_path.read_text(encoding="utf-8").strip(),
                "compiled_stats": compiled.stats(),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
