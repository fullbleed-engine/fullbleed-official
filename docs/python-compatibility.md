# Python compatibility checks

Fullbleed's published CPython wheels use the stable ABI starting at Python 3.10.
The stable-version CI matrix covers Python 3.10 through 3.14. Installation needs
no Python runtime dependencies, browser, or Rust compiler.

## Python 3.15 preview

The separate **Python Preview** jobs pin **CPython 3.15.0rc3**, with the standard
GIL, on Windows x64 and Linux x64. They install the exact same wheel produced
and tested by the Python 3.10 build job. This is preview coverage; the supported
stable-version list and package classifiers remain unchanged.

Each job checks the installed API, CLI, packaged agent contract, and strict
doctor command, then runs the repository Python suite against the installed
native extension. The suite intentionally imports the checkout's Python wrappers;
the separate installed-package checks run with `-I` isolation.

The jobs also render the border-image, Standard 14 font, and gradient regression
fixtures. Every retained PDF and PNG must match the corresponding Python 3.10
output byte for byte. Missing files, failed reports, engine-version differences,
test failures, and unexpected skipped tests fail the check.

One explicit test exception remains: the legacy subinterpreter test imports
`_xxsubinterpreters`, which this preview does not provide. Its skip and reason
are retained in the report rather than counted as a pass. These checks do not
establish support for free-threaded builds, subinterpreters, Python 3.15 final,
or macOS on this preview.

Open the [CI runs](https://github.com/fullbleed-engine/fullbleed-official/actions/workflows/ci.yml)
and inspect both **Python Preview** jobs for the commit you use. Download their
`python-prerelease-Windows-3.15.0rc3` and
`python-prerelease-Linux-3.15.0rc3` artifacts for interpreter identity, JUnit test
results, PDF/PNG files, and the comparison's `verification.json`.

To reproduce on that exact preview, install a wheel and the test tools, run the
commands in [the CI workflow](../.github/workflows/ci.yml), and download the
matching stable-run reference artifacts from the same commit. The final check is:

```sh
python -I tools/check_python_compatibility.py \
  --expected-python 3.15.0rc3 --expected-version 2.5.15 \
  --reference target/python-stable-reference --out target/python-prerelease \
  --allow-skipped-test tests.test_fullbleed_python_abi.test_stable_abi_module_can_initialize_in_a_subinterpreter
```

The [official rc3 release page](https://www.python.org/downloads/release/python-3150rc3/)
describes this prerelease. Results for one preview cannot establish compatibility
with a later interpreter release.
