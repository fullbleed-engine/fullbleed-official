# IronPress parity harness

FullBleed uses the independent [IronPress](https://github.com/gastongouron/ironpress)
CSS-to-PDF corpus as a pinned compatibility check. The harness is
pinned to IronPress commit
`0d1e53b6d8174d0a5059a8696c24e62759381f6d` (MIT).

The integration changes only IronPress's candidate-renderer boundary. Its 1,662
HTML fixtures, manifests, committed browser PDF oracles, Poppler rasterization,
visibility comparator, and report generator remain byte-for-byte upstream. The
tracked candidate patch is `tools/ironpress_fullbleed.patch`; the FullBleed side
is `tools/ironpress_fullbleed_adapter.py`.

## Reproduce

Build or select a CPython 3.10 stable-ABI Linux x86-64 wheel inside this
repository. Both local `linux_x86_64` and published `manylinux` wheels are
accepted; the filename and package metadata must identify the same Fullbleed
version. Substitute your actual wheel filename in these commands.

Run the six substrate probes:

```powershell
python tools/run_ironpress_parity.py `
  --only probes `
  --wheel target/ironpress-wheel/fullbleed-2.5.14-cp310-abi3-linux_x86_64.whl `
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
  --wheel target/ironpress-wheel/fullbleed-2.5.14-cp310-abi3-linux_x86_64.whl `
  --keep-pdfs `
  --evidence-dir target/ironpress-evidence/full
```

Use a new or empty evidence directory for each invocation. Existing evidence
is never overwritten. The container selects Rust 1.97, Ubuntu 24.04, and
Poppler 24.08.0, including the `pdftoppm` binary checksum. The upstream library
revision does not commit a Cargo lockfile. `tools/ironpress.Cargo.lock` locks
its unchanged dependency declarations, resolved with Cargo 1.97, and the
comparator builds with `--locked`. The runner verifies the upstream commit, patch hash, and
modified-file boundary before rendering. Image tags and operating-system
packages are not a byte-identical environment lock; the actual image ID is
retained for each run.

## Inspect and retain the evidence

Open `reports/index.html` inside the exported directory. The export includes
`refs/`, `out/`, `diffs/`, and the report's own assets, preserving its image
links. `--keep-pdfs` also exports candidate PDFs in `pdfs/`. The upstream
report retains its IronPress headings and candidate labels; **the candidate
renderer in this integration is Fullbleed**, identified by its wheel below.
The reference PDFs are the committed upstream oracles.

`fullbleed-run.json` records the tested wheel's version and SHA-256, upstream
revision, adapter and patch hashes, image ID, invocation ID, scope, exit code,
and report verification result. `manifest.json` inventories the exported
files and their SHA-256 values. The exported `Cargo.lock` preserves the
comparator's dependency resolution; `IRONPRESS-LICENSE` preserves attribution.
The runner rejects a mismatched invocation,
an incomplete or duplicate fixture inventory, inconsistent verdict counts,
or Markdown/HTML belonging to a different JSON report. It preserves upstream
failures and disputed references without changing their classifications.

`gate_passed: true` requires an unfiltered, complete 1,662-fixture report,
zero failing fixtures, an upstream gate exit of zero, and a successful export.
A complete report can still contain failures; `verified_complete` describes
the evidence, not a passing comparison. A filtered run cannot pass this gate.
These results do not establish compatibility with every CSS feature or live
browser behavior.

The [Independent CSS corpus workflow](../.github/workflows/ironpress-parity.yml)
defaults to `package_source: published` and compares the exact `package_version`
from PyPI. Select `package_source: source` to build an unpublished candidate
from the chosen Git ref; `package_version` is ignored in that mode. Pull
requests affecting the runner, adapter, or border renderer use source mode.
Source mode uses Rust 1.97 on Ubuntu 24.04, matching the comparator container's
operating-system release. The extra `ironpress-input.json` identifies the input
kind, checked-out commit, and wheel hash; the exact wheel is also retained.
A source wheel's unchanged version number is not evidence that it is the
published package. Use its commit and hash to distinguish candidate results.

Download the artifact to inspect the report, candidate PDFs, and log, including
failed runs. Workflow artifacts expire after seven days; retain a downloaded copy
and its manifest before using a result in a lasting compatibility claim.

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
