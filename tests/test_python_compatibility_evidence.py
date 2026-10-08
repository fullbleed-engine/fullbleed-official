"""A readiness check must reject missing output and silently skipped coverage."""
import json

import pytest

from tools.check_python_compatibility import RENDER_CHECKS, check_test_results, compare_renderings


def test_render_comparison_requires_the_same_complete_output(tmp_path):
    reference, candidate = tmp_path / "stable", tmp_path / "preview"
    for root in (reference, candidate):
        for check in RENDER_CHECKS:
            folder = root / check
            folder.mkdir(parents=True)
            (folder / "verification.json").write_text(json.dumps({"ok": True, "version": "1.2.3"}))
            (folder / "output.pdf").write_bytes(b"%PDF-test")
            (folder / "preview.png").write_bytes(b"PNG-test")
    assert sum(row["files"] for row in compare_renderings(reference, candidate, "1.2.3").values()) == 6
    changed = candidate / RENDER_CHECKS[0] / "preview.png"
    changed.write_bytes(b"changed pixels")
    with pytest.raises(ValueError, match="bytes differ"):
        compare_renderings(reference, candidate, "1.2.3")
    changed.unlink()
    with pytest.raises(ValueError, match="Missing or unexpected"):
        compare_renderings(reference, candidate, "1.2.3")


@pytest.mark.parametrize("record,allowed,message", [
    ('<failure message="render failed"/>', set(), "failure or error"),
    ('<skipped message="reader not installed"/>', set(), "Unexpected skipped"),
])
def test_test_evidence_rejects_failures_and_unapproved_skips(tmp_path, record, allowed, message):
    path = tmp_path / "tests.xml"
    path.write_text('<testsuites><testsuite tests="1" failures="0" errors="0">'
                    '<testcase classname="tests.render" name="output">' + record +
                    '</testcase></testsuite></testsuites>')
    with pytest.raises(ValueError, match=message):
        check_test_results(path, allowed)


def test_explicit_skip_is_reported_as_skipped_not_passed(tmp_path):
    path = tmp_path / "tests.xml"
    path.write_text('<testsuites><testsuite tests="2" failures="0" errors="0" skipped="1">'
                    '<testcase classname="tests.render" name="output"/>'
                    '<testcase classname="tests.abi" name="legacy"><skipped message="API absent"/>'
                    '</testcase></testsuite></testsuites>')
    result = check_test_results(path, {"tests.abi.legacy"})
    assert result == {"tests": 2, "passed": 1, "skipped": [{"test": "tests.abi.legacy", "reason": "API absent"}]}
