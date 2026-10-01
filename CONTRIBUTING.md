# Contributing to Fullbleed

Fullbleed is an MIT-licensed print-document engine. Contributions can be small:
a reproducible layout bug, a clearer guide, a document example, or a focused fix.

Start with the [quickstart](https://docs.fullbleed.dev/getting-started/quickstart/)
and [example PDFs](https://docs.fullbleed.dev/examples/). Ask usage questions in
[Discussions](https://github.com/fullbleed-engine/fullbleed-official/discussions).
Use [issues](https://github.com/fullbleed-engine/fullbleed-official/issues/new/choose)
for reproducible bugs and feature proposals. Search existing threads first.

## Report a useful bug

Include the Fullbleed and Python versions, operating system and architecture,
the smallest runnable Python/HTML/CSS input, expected output, and actual output.
For layout problems, attach a screenshot or PDF and identify the affected page.
Use synthetic data and assets you can share publicly. A small failing document
is more useful than a large application that we cannot run.

Useful diagnostics:

```bash
python -m fullbleed --version
python -m fullbleed doctor --json
python -m fullbleed inspect pdf output.pdf --json
```

Fullbleed implements static document layout. Check the [CSS coverage](docs/css-coverage.md)
when reporting a difference from a browser. For profile or accessibility issues,
include the validator name/version, exact profile, and report when available.

## Set up a source checkout

Use Python 3.10 or newer and a stable Rust toolchain with Cargo. Native builds
also need the platform's C/C++ linker tools. See the
[Rust installation guide](https://www.rust-lang.org/tools/install) for setup.

```bash
git clone https://github.com/fullbleed-engine/fullbleed-official.git
cd fullbleed-official
python -m venv .venv
```

Activate with `.venv\Scripts\Activate.ps1` in PowerShell or
`source .venv/bin/activate` on macOS/Linux. Then build and install the current
checkout into that environment:

```bash
python -m pip install --upgrade pip pytest
python -m pip install --no-deps --force-reinstall .
python -m fullbleed doctor --strict --json
```

The install builds a wheel through the in-tree build backend. Repeat the install
after native changes so tests do not load an older extension. The core runtime
does not require pytest or other development tools.

## Make and check a change

Keep each pull request focused on one behavior. Add a regression test when fixing
an engine or CLI bug; documentation corrections usually only need their examples
and links checked. Run the relevant tests first, then broaden validation as needed.

For Python tests, select the newly installed native engine explicitly:

```powershell
# PowerShell
$env:FULLBLEED_TEST_INSTALLED_NATIVE = "1"
python -m pytest tests/test_fullbleed_cli_pdf_profiles.py -q
```

```bash
# macOS/Linux shell
FULLBLEED_TEST_INSTALLED_NATIVE=1 python -m pytest tests/test_fullbleed_cli_pdf_profiles.py -q
```

Replace the example test path with the tests for your change. Run the full Python
suite with `python -m pytest -q` while the same environment variable is set.
For Rust changes, run:

```bash
cargo fmt --all --check
cargo test --locked
cargo test --locked --features python,svg_raster
```

CI also builds and installs wheels, checks supported Python versions, runs
document examples and goldens, and retains print-profile verification evidence.
Describe which local checks passed in your PR, including anything you could not run.

## Find the right part of the project

| Area | Location |
| --- | --- |
| Rust layout and PDF engine | `src/` |
| Python API and UI components | `python/fullbleed/` |
| Python CLI | `python/fullbleed_cli/` |
| Python tests and document goldens | `tests/`, `goldens/` |
| Runnable document projects | `examples/` |
| Optional MCP integration | `packages/fullbleed-mcp/` |
| Published website guides | [fullbleed-engine/docs](https://github.com/fullbleed-engine/docs) |

Preserve deterministic output and structured diagnostics. Keep browsers, system
PDF stacks, system-font requirements, and AI-vendor dependencies out of the core.
Keep optional integration dependencies in their own packages.

Capability facts and parser schemas belong in shared runtime definitions. Do not
hand-edit `fullbleed-agent-contract.json`, `cli_schema.md`, or `llms.txt`. After
building and installing your changed wheel, regenerate them with:

```bash
python tools/generate_agent_contract.py --python python --json
python tools/generate_agent_contract.py --python python --check --json
```

For bundled Skill changes, use the validator and publishing dry run documented in
[agent discovery](docs/agent-discovery.md). Release procedures and the limits of
conformance claims are recorded in [release documentation](docs/release/README.md).
Keep claims tied to retained evidence.

Contributions are distributed under the repository's [MIT license](LICENSE).
