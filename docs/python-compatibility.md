# Python compatibility checks

Fullbleed's published CPython wheels use the stable ABI starting at Python 3.10.
The supported range is Python 3.10–3.15 with the standard GIL. Installation needs
no Python runtime dependencies, browser, or Rust compiler.

## Python 3.15 final

The [CI workflow](../.github/workflows/ci.yml) pins **CPython 3.15.0 final** for
the detailed compatibility checks. There are two independent paths:

| Package under test | Source and wheel | Platforms |
| --- | --- | --- |
| Development build | The same wheel built and tested on Python 3.10 in that CI run | Windows x64, Linux x64 |
| Public stable **2.5.34** | Wheel downloaded from PyPI; tests pinned to release commit `0e64857b103522fcb626a81c38306e175106fc67` | Windows x64, Linux x64, macOS Intel and Apple silicon |

The public-package jobs verify the downloaded wheel's SHA-256 against PyPI and
record its exact URL, stable-ABI tags, dependency metadata, source commit, and
baseline interpreter. They install that same wheel file on both Python versions.
The development jobs download their Python 3.10 reference outputs and wheel from
the current CI run.

Each path exercises the installed API, CLI, agent contract, and strict doctor
command in isolation. The full Python suite uses the matching source wrappers
and the installed native extension. A published wheel is tested with its own
release's suite, so an older package is not assessed using newer alpha APIs.

The border-image, filtered-overflow, Standard 14 font, and gradient fixtures are
rendered on Python 3.10 and 3.15. Every retained PDF and PNG must match byte for
byte. Missing files, unsuccessful fixture reports, engine-version differences,
test failures, and unexpected skipped tests fail the check.

One explicit test exception remains: the legacy subinterpreter test imports
`_xxsubinterpreters`, which these Python 3.15 builds do not provide. Its skip and
reason remain in the report; they are not counted as a pass. These results cover
standard-GIL CPython. They do not establish support for free-threaded builds,
subinterpreters, other Python implementations, or every operating-system and
architecture combination.

The ordinary Linux compatibility matrix covers Python 3.10–3.15. Release CI also
installs the manylinux x64 wheel in each supported Python minor version before
publication; platform-specific installation checks cover the other published
wheel targets. The public Linux job also exercises its wheel in the official
`python:3.15-slim` image with networking disabled, matching the release
container's installation and CLI checks.

## Retained evidence

Open the [CI runs](https://github.com/fullbleed-engine/fullbleed-official/actions/workflows/ci.yml)
and select the commit you use:

- **Python 3.15** jobs retain `python315-Windows` and `python315-Linux`.
- **Published Python 3.15** jobs retain `python315-published-<runner-label>`.
  Each includes `source.json`, both pip installation reports, Python 3.10
  reference output, and the Python 3.15 candidate's tests and renderings.
- The candidate's `verification.json` records interpreter identity, installed
  package paths, test totals and explicit skips, and the hashes of every
  compared PDF and PNG.

For the public stable artifacts, the final comparison can be repeated using
Python 3.15.0 with `fullbleed==2.5.34` installed and the downloaded `reference`
and `candidate` folders:

```sh
python -I tools/check_python_compatibility.py \
  --expected-python 3.15.0 --expected-version 2.5.34 \
  --reference reference --out candidate \
  --allow-skipped-test tests.test_fullbleed_python_abi.test_stable_abi_module_can_initialize_in_a_subinterpreter
```

## Earlier release-candidate checks

The [original rc3 run](https://github.com/fullbleed-engine/fullbleed-official/actions/runs/37720487567)
and [2.5.15 release evidence](https://github.com/fullbleed-engine/fullbleed-official/releases/tag/v2.5.15)
remain historical records. They used CPython 3.15.0rc3 on Windows and Linux.
The final-release checks above replace that preview coverage.

Python's [official 3.15.0 release page](https://www.python.org/downloads/release/python-3150/)
identifies the final interpreter release.
