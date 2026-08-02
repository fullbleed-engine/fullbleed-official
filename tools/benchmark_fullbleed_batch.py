#!/usr/bin/env python3
"""Reproducible FullBleed single-call and native batch throughput benchmark."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Callable, Sequence


CSS = r"""
@page { size: 612pt 792pt; margin: 24pt; }
* { box-sizing: border-box; }
body { margin: 0; font-family: Helvetica, sans-serif; font-size: 10pt; color: #172033; }
.sheet { border: 1pt solid #1f3a5f; padding: 12pt; background: #f8fafc; }
.head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10pt; }
.title { font-size: 20pt; line-height: 1.1; font-weight: 700; color: #102a43; }
.badge { padding: 4pt 7pt; border-radius: 3pt; background: #0b7285; color: white; }
.grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6pt; }
.card { min-height: 48pt; padding: 6pt; border: 0.75pt solid #9fb3c8; background: white; }
.label { font-size: 8pt; color: #627d98; text-transform: uppercase; letter-spacing: .3pt; }
.value { margin-top: 3pt; font-size: 13pt; font-weight: 700; }
table { width: 100%; margin-top: 10pt; border-collapse: collapse; table-layout: fixed; }
th, td { padding: 4pt 5pt; border-bottom: .5pt solid #bcccdc; text-align: right; }
th:first-child, td:first-child { text-align: left; }
tbody tr:nth-child(even) { background: #edf2f7; }
.negative { color: #c92a2a; }
.positive { color: #2b8a3e; }
"""


def installed_fullbleed_version() -> str:
    try:
        return version("fullbleed")
    except PackageNotFoundError:
        return "source-checkout"


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def documents(count: int) -> list[str]:
    result: list[str] = []
    for document_index in range(count):
        rows = []
        balance = 10_000 + document_index * 17
        for row_index in range(24):
            amount = ((document_index + 3) * (row_index + 11)) % 997 - 430
            balance += amount
            value_class = "positive" if amount >= 0 else "negative"
            rows.append(
                f"<tr><td>Entry {row_index + 1:02d}</td><td class='{value_class}'>"
                f"{amount:+d}.00</td><td>{balance}.00</td></tr>"
            )
        result.append(
            "<!doctype html><html><body><main class='sheet'>"
            "<header class='head'>"
            f"<h1 class='title'>Account {document_index:05d}</h1>"
            f"<span class='badge'>Batch {document_index % 13:02d}</span>"
            "</header><section class='grid'>"
            f"<article class='card'><div class='label'>Opening</div><div class='value'>{10_000 + document_index * 17}.00</div></article>"
            f"<article class='card'><div class='label'>Entries</div><div class='value'>{len(rows)}</div></article>"
            f"<article class='card'><div class='label'>Closing</div><div class='value'>{balance}.00</div></article>"
            "</section><table><thead><tr><th>Description</th><th>Amount</th><th>Balance</th></tr></thead>"
            f"<tbody>{''.join(rows)}</tbody></table></main></body></html>"
        )
    return result


def measure(operation: Callable[[], int], repeats: int, document_count: int) -> dict[str, object]:
    samples: list[float] = []
    output_bytes = 0
    for _ in range(repeats):
        started = time.perf_counter()
        output_bytes = operation()
        samples.append(time.perf_counter() - started)
    median = statistics.median(samples)
    return {
        "seconds": samples,
        "median_seconds": median,
        "documents_per_second": document_count / median,
        "output_bytes": output_bytes,
    }


def worker(arguments: argparse.Namespace) -> int:
    os.environ["FULLBLEED_THREADS"] = str(arguments.threads)
    import fullbleed

    html_list = documents(arguments.documents)
    engine = fullbleed.PdfEngine()
    engine.render_pdf(html_list[0], CSS)

    with tempfile.TemporaryDirectory(prefix="fullbleed-batch-bench-") as directory:
        output_path = Path(directory) / "batch.pdf"

        reference_pdf = bytes(engine.render_pdf_batch(html_list, CSS))
        parallel_pdf = bytes(engine.render_pdf_batch_parallel(html_list, CSS))
        if parallel_pdf != reference_pdf:
            raise RuntimeError("parallel buffer output differs from sequential output")
        engine.render_pdf_batch_to_file(html_list, CSS, str(output_path))
        if output_path.read_bytes() != reference_pdf:
            raise RuntimeError("sequential file output differs from buffer output")
        engine.render_pdf_batch_to_file_parallel(html_list, CSS, str(output_path))
        if output_path.read_bytes() != reference_pdf:
            raise RuntimeError("parallel file output differs from sequential output")

        def single_loop() -> int:
            return sum(len(bytes(engine.render_pdf(html, CSS))) for html in html_list)

        def sequential_buffer() -> int:
            return len(bytes(engine.render_pdf_batch(html_list, CSS)))

        def parallel_buffer() -> int:
            return len(bytes(engine.render_pdf_batch_parallel(html_list, CSS)))

        def sequential_file() -> int:
            return int(engine.render_pdf_batch_to_file(html_list, CSS, str(output_path)))

        def parallel_file() -> int:
            return int(
                engine.render_pdf_batch_to_file_parallel(
                    html_list, CSS, str(output_path)
                )
            )

        operations: list[tuple[str, Callable[[], int]]] = [
            ("single_loop", single_loop),
            ("batch_sequential_buffer", sequential_buffer),
            ("batch_parallel_buffer", parallel_buffer),
            ("batch_sequential_file", sequential_file),
            ("batch_parallel_file", parallel_file),
        ]
        results = {
            name: measure(operation, arguments.repeats, arguments.documents)
            for name, operation in operations
        }

    payload = {
        "schema": "fullbleed.batch_benchmark.v1",
        "threads": arguments.threads,
        "documents": arguments.documents,
        "repeats": arguments.repeats,
        "logical_cpus": os.cpu_count(),
        "fullbleed_version": installed_fullbleed_version(),
        "workload_sha256": sha256_bytes(
            (CSS + "\0" + "\0".join(html_list)).encode("utf-8")
        ),
        "batch_pdf_sha256": sha256_bytes(reference_pdf),
        "python": sys.version.split()[0],
        "platform": sys.platform,
        "results": results,
    }
    print(json.dumps(payload, sort_keys=True))
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--documents", type=int, default=128)
    result.add_argument("--repeats", type=int, default=3)
    result.add_argument("--threads", type=int, nargs="+", default=[1, 2, 4, 8])
    result.add_argument("--output", type=Path)
    result.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    if arguments.documents < 1 or arguments.repeats < 1:
        raise SystemExit("--documents and --repeats must be positive")
    if any(thread_count < 1 for thread_count in arguments.threads):
        raise SystemExit("--threads values must be positive")
    if arguments.worker:
        if len(arguments.threads) != 1:
            raise SystemExit("worker mode requires exactly one thread count")
        arguments.threads = arguments.threads[0]
        return worker(arguments)

    rows = []
    for thread_count in arguments.threads:
        command = [
            sys.executable,
            str(Path(__file__).resolve()),
            "--worker",
            "--documents",
            str(arguments.documents),
            "--repeats",
            str(arguments.repeats),
            "--threads",
            str(thread_count),
        ]
        completed = subprocess.run(command, check=False, text=True, capture_output=True)
        if completed.returncode != 0:
            sys.stdout.write(completed.stdout)
            sys.stderr.write(completed.stderr)
            return completed.returncode
        rows.append(json.loads(completed.stdout))

    rendered = json.dumps(
        {
            "schema": "fullbleed.batch_benchmark_matrix.v1",
            "runs": rows,
        },
        indent=2,
        sort_keys=True,
    )
    if arguments.output is not None:
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
