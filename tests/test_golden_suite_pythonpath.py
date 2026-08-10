import os
from pathlib import Path

from goldens import run_golden_suite


def _parts(value):
    return [] if value is None else value.split(os.pathsep)


def test_case_pythonpath_prefers_source_when_extension_available():
    other = str(run_golden_suite.ROOT / "other_python")

    value = run_golden_suite._case_pythonpath(
        other,
        source_extension_available=True,
    )

    parts = _parts(value)
    assert Path(parts[0]).resolve() == run_golden_suite.SOURCE_PYTHON_DIR.resolve()
    assert parts[1:] == [other]


def test_case_pythonpath_removes_source_when_extension_missing():
    source = str(run_golden_suite.SOURCE_PYTHON_DIR)
    other = str(run_golden_suite.ROOT / "other_python")
    existing = os.pathsep.join([source, other])

    value = run_golden_suite._case_pythonpath(
        existing,
        source_extension_available=False,
    )

    parts = _parts(value)
    assert parts == [other]


def test_case_pythonpath_clears_source_only_path_when_extension_missing():
    value = run_golden_suite._case_pythonpath(
        str(run_golden_suite.SOURCE_PYTHON_DIR),
        source_extension_available=False,
    )

    assert value is None


def test_installed_native_switch_ignores_source_extension(monkeypatch):
    monkeypatch.setenv("FULLBLEED_TEST_INSTALLED_NATIVE", "1")

    assert run_golden_suite._source_extension_available() is False
