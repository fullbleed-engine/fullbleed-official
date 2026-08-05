#!/usr/bin/env python3
"""Benchmark compiled fixed-geometry variable-data PDF generation.

Unlike the immutable-copy benchmark, every output page has a unique invoice ID and
five additional record bindings. HTML parsing and layout run once; the timed path
includes Python-to-Rust column conversion and generation of every distinct page.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import re
import statistics
import sys
import time
from pathlib import Path

import fullbleed


HTML = """<!doctype html>
<html lang="en">
<body>
  <main class="invoice">
    <header>
      <h1>FULLBLEED INDUSTRIES</h1>
      <p class="muted">Performance systems and document infrastructure</p>
      <p>Invoice: {{invoice_id}}</p>
      <p>Account: {{account_id}}</p>
    </header>
    <section>
      <h2>Bill to</h2>
      <p>Customer: {{customer_name}}</p>
      <p>Payment reference: {{payment_ref}}</p>
      <p>Due date: {{due_date}}</p>
    </section>
    <section>
      <h2>Services</h2>
      <p>Compiled document processing platform</p>
      <p>Variable-data rendering and delivery</p>
      <p>Managed production support</p>
      <p class="total">Balance due: {{balance}}</p>
    </section>
    <footer>
      <p>Thank you for your business.</p>
      <p class="muted">Static page paint is shared; record text is bound once per page.</p>
    </footer>
  </main>
</body>
</html>
"""


CSS = """
@page { size: letter; margin: 0.45in; }
body {
  margin: 0;
  color: #172033;
  font-family: Helvetica, sans-serif;
  font-size: 10pt;
  line-height: 1.35;
}
.invoice { border: 1.5pt solid #243b64; padding: 22pt; }
header { border-bottom: 2pt solid #376cb8; padding-bottom: 12pt; }
h1 { margin: 0 0 4pt; color: #173969; font-size: 22pt; }
h2 { margin: 16pt 0 6pt; color: #244f86; font-size: 13pt; }
p { margin: 3pt 0; }
.muted { color: #67738a; }
.total {
  margin-top: 14pt;
  border-top: 1pt solid #aebbd0;
  padding-top: 9pt;
  color: #173969;
  font-size: 14pt;
}
footer { margin-top: 28pt; border-top: 1pt solid #d8deea; padding-top: 10pt; }
"""


def make_columns(records: int) -> dict[str, list[str]]:
    return {
        "invoice_id": [f"INV-{row:09d}" for row in range(records)],
        "account_id": [f"ACCT-{row * 7_919:012d}" for row in range(records)],
        "customer_name": [f"Customer {row:09d}" for row in range(records)],
        "payment_ref": [f"PAY-{row * 104_729:015d}" for row in range(records)],
        "due_date": [
            f"{2027 + row // 365:04d}-{1 + (row // 31) % 12:02d}-{1 + row % 28:02d}"
            for row in range(records)
        ],
        "balance": [
            f"${1_000 + row * 37 // 100:,}.{row * 37 % 100:02d}"
            for row in range(records)
        ],
    }


def input_sha256(columns: dict[str, list[str]]) -> str:
    digest = hashlib.sha256()
    for name in sorted(columns):
        digest.update(name.encode("ascii"))
        digest.update(b"\0")
        for value in columns[name]:
            digest.update(value.encode("utf-8"))
            digest.update(b"\0")
    return digest.hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def verify_output(pdf: bytes, records: int) -> dict[str, object]:
    page_count = pdf.count(b"/Type /Page ")
    if page_count != records:
        raise RuntimeError(f"expected {records} PDF pages, found {page_count}")
    if b"{{" in pdf or b"}}" in pdf:
        raise RuntimeError("unresolved binding marker found in output")

    pattern = re.compile(rb"INV-\d{9}")
    seen = 0
    for seen, match in enumerate(pattern.finditer(pdf), start=1):
        expected = f"INV-{seen - 1:09d}".encode("ascii")
        if match.group(0) != expected:
            raise RuntimeError(
                f"page {seen} has {match.group(0)!r}; expected {expected!r}"
            )
    if seen != records:
        raise RuntimeError(f"verified {seen} unique invoice IDs; expected {records}")

    contents_pattern = re.compile(rb"/Contents \[(\d+) 0 R (\d+) 0 R\]")
    static_content_id: bytes | None = None
    dynamic_content_ids: set[bytes] = set()
    content_arrays = 0
    for content_arrays, match in enumerate(contents_pattern.finditer(pdf), start=1):
        if static_content_id is None:
            static_content_id = match.group(1)
        elif match.group(1) != static_content_id:
            raise RuntimeError("page batch does not share one static content stream")
        dynamic_content_ids.add(match.group(2))
    if content_arrays != records or len(dynamic_content_ids) != records:
        raise RuntimeError(
            "expected one shared static stream and one unique dynamic stream per page"
        )

    return {
        "page_count": page_count,
        "unique_invoice_ids_verified": seen,
        "first_invoice_id": "INV-000000000",
        "last_invoice_id": f"INV-{records - 1:09d}",
        "markers_resolved": True,
        "shared_static_content_streams": 1,
        "unique_dynamic_content_streams": len(dynamic_content_ids),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records", type=int, default=100_000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/benchmarks/fullbleed-vdp-100000.pdf"),
    )
    parser.add_argument("--no-write", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.records <= 0 or args.repeats <= 0:
        raise SystemExit("--records and --repeats must be positive")

    columns = make_columns(args.records)
    if len(set(columns["invoice_id"])) != args.records:
        raise RuntimeError("input invoice IDs are not unique")
    source_digest = input_sha256(columns)

    engine = fullbleed.PdfEngine()
    compile_started = time.perf_counter()
    compiled = engine.compile_pdf(HTML, CSS)
    compile_seconds = time.perf_counter() - compile_started
    stats = compiled.stats()
    expected_slots = sorted(columns)
    if stats.get("binding_slots") != expected_slots:
        raise RuntimeError(
            f"compiled slots {stats.get('binding_slots')!r} do not match {expected_slots!r}"
        )

    warm_count = min(args.records, 128)
    compiled.render_pdf_bindings(
        {name: values[:warm_count] for name, values in columns.items()}
    )

    render_samples: list[float] = []
    output_hashes: list[str] = []
    pdf = b""
    for _ in range(args.repeats):
        # Drop the prior large output before evaluating the next call so the benchmark does not
        # require two complete PDFs to coexist merely to gather repeated samples.
        pdf = b""
        render_started = time.perf_counter()
        pdf = compiled.render_pdf_bindings(columns)
        render_samples.append(time.perf_counter() - render_started)
        output_hashes.append(hashlib.sha256(pdf).hexdigest())
    if len(set(output_hashes)) != 1:
        raise RuntimeError("repeated binding renders were not byte deterministic")
    render_seconds = statistics.median(render_samples)
    verification = verify_output(pdf, args.records)
    output_bytes = len(pdf)
    output_sha256 = output_hashes[0]

    direct_file_samples: list[float] = []
    direct_file_hashes: list[str] = []
    output_path: str | None = None
    if not args.no_write:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        # The buffer result is no longer needed; release it before exercising the streaming lane.
        pdf = b""
        for _ in range(args.repeats):
            write_started = time.perf_counter()
            written = compiled.render_pdf_bindings_to_file(columns, str(args.output))
            direct_file_samples.append(time.perf_counter() - write_started)
            if written != output_bytes or args.output.stat().st_size != output_bytes:
                raise RuntimeError(
                    "direct-file byte count differs from the buffer render"
                )
            direct_file_hashes.append(file_sha256(args.output))
        if len(set(direct_file_hashes)) != 1 or direct_file_hashes[0] != output_sha256:
            raise RuntimeError(
                "direct-file output differs from the deterministic buffer render"
            )
        output_path = str(args.output.resolve())

    direct_file_seconds = (
        statistics.median(direct_file_samples) if direct_file_samples else None
    )

    result = {
        "benchmark": "compiled-fixed-geometry-variable-data-v1",
        "records": args.records,
        "repeats": args.repeats,
        "pages": args.records,
        "variable_fields_per_record": len(columns),
        "all_records_distinct": True,
        "compile_seconds": compile_seconds,
        "render_seconds": render_seconds,
        "render_seconds_samples": render_samples,
        "render_seconds_min": min(render_samples),
        "render_seconds_max": max(render_samples),
        "pages_per_second": args.records / render_seconds,
        "total_including_compile_seconds": compile_seconds + render_seconds,
        "pages_per_second_including_compile": args.records
        / (compile_seconds + render_seconds),
        "direct_file_seconds": direct_file_seconds,
        "direct_file_seconds_samples": direct_file_samples,
        "direct_file_pages_per_second": (
            args.records / direct_file_seconds if direct_file_seconds else None
        ),
        "direct_file_pages_per_second_including_compile": (
            args.records / (compile_seconds + direct_file_seconds)
            if direct_file_seconds
            else None
        ),
        "direct_file_output_byte_deterministic": (
            len(set(direct_file_hashes)) == 1 if direct_file_hashes else None
        ),
        "output_bytes": output_bytes,
        "output_megabytes": output_bytes / 1_000_000,
        "output_sha256": output_sha256,
        "repeated_output_byte_deterministic": True,
        "input_sha256": source_digest,
        "output_path": output_path,
        "compiled_stats": stats,
        "verification": verification,
        "python": sys.version.split()[0],
        "platform": platform.platform(),
        "processor": platform.processor(),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
