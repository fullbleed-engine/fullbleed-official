# IronPress parity harness

FullBleed uses the independent [IronPress](https://github.com/gastongouron/ironpress)
CSS-to-PDF corpus as an exact, reproducible compatibility check. The harness is
pinned to IronPress commit
`0d1e53b6d8174d0a5059a8696c24e62759381f6d` (MIT).

The integration changes only IronPress's candidate-renderer boundary. Its 1,662
HTML fixtures, manifests, committed browser PDF oracles, Poppler rasterization,
visibility comparator, and report generator remain byte-for-byte upstream. The
tracked candidate patch is `tools/ironpress_fullbleed.patch`; the FullBleed side
is `tools/ironpress_fullbleed_adapter.py`.

## Reproduce

Build or select a CPython 3.10 stable-ABI Linux x86-64 wheel, then run the six
substrate probes:

```powershell
python tools/run_ironpress_parity.py `
  --only probes `
  --wheel target/ironpress-wheel/fullbleed-2.2.3-cp310-abi3-linux_x86_64.whl `
  --keep-pdfs `
  --evidence-dir target/ironpress-evidence/probes
```

Filtered runs are diagnostics. IronPress intentionally returns a nonzero status
even when every selected fixture passes because a filter cannot satisfy the
full-corpus gate. The diagnostic images are still copied to the evidence
directory.

Run the complete corpus by omitting `--only`:

```powershell
python tools/run_ironpress_parity.py `
  --wheel target/ironpress-wheel/fullbleed-2.2.3-cp310-abi3-linux_x86_64.whl `
  --evidence-dir target/ironpress-evidence/full
```

The container pins Rust 1.97, Ubuntu 24.04, and Poppler 24.08.0, including the
`pdftoppm` binary checksum. The runner verifies the upstream commit, patch hash,
and modified-file boundary before rendering.

## Adapter and compute model

IronPress uses at most eight Rayon fixture workers. Each worker owns one
persistent framed Python adapter process and reuses one `PdfEngine` and font
registry instead of starting Python for every fixture. Adapter processes use
`FULLBLEED_THREADS=1`: fixture-level concurrency already occupies the machine,
so this prevents nested oversubscription.

Production batch rendering does not use Python multiprocessing. The Python API
releases the GIL, then FullBleed runs ordered scoped Rust workers. Nested native
parallel regions automatically collapse to one worker. The streaming batch path
uses a bounded channel (`min(threads * 4, 256)`) and one ordered PDF writer, so
memory is bounded while rendering stays parallel and output remains
deterministic.

Run the batch benchmark against an installed wheel:

```powershell
python tools/benchmark_fullbleed_batch.py `
  --documents 512 `
  --repeats 5 `
  --threads 1 2 4 8 16 `
  --output target/ironpress-evidence/batch-benchmark.json
```

The benchmark compares individual calls, sequential combined-PDF paths, and
parallel combined-PDF buffer/file paths using the same generated 24-row document
workload.
