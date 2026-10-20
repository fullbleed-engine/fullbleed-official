"""A readiness check must reject missing output and silently skipped coverage."""
import json
import hashlib
import zipfile

import pytest

from tools.check_python_compatibility import RENDER_CHECKS, check_test_results, compare_renderings
from tools.record_python_compatibility_source import verify_public_wheel, verify_source_version


def test_render_comparison_requires_the_same_complete_output(tmp_path):
    reference, candidate = tmp_path / "stable", tmp_path / "preview"
    for root in (reference, candidate):
        for check in RENDER_CHECKS:
            folder = root / check
            folder.mkdir(parents=True)
            (folder / "verification.json").write_text(json.dumps({"ok": True, "version": "1.2.3"}))
            (folder / "output.pdf").write_bytes(b"%PDF-test")
            (folder / "preview.png").write_bytes(b"PNG-test")
    assert sum(row["files"] for row in compare_renderings(reference, candidate, "1.2.3").values()) == 8
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


def _public_wheel(tmp_path, *, dependencies=""):
    path = tmp_path / "fullbleed-1.2.3-cp310-abi3-win_amd64.whl"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(
            "fullbleed-1.2.3.dist-info/METADATA",
            "Metadata-Version: 2.4\nName: fullbleed\nVersion: 1.2.3\n"
            "Requires-Python: >=3.10\n" + dependencies)
        archive.writestr("fullbleed-1.2.3.dist-info/WHEEL",
                         "Wheel-Version: 1.0\nTag: cp310-abi3-win_amd64\n")
    release = {
        "info": {"name": "fullbleed", "version": "1.2.3"},
        "urls": [{"filename": path.name, "digests": {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()},
            "url": "https://files.pythonhosted.org/test/fullbleed.whl"}],
    }
    return path, release


def test_public_wheel_evidence_rejects_changed_download_and_wrong_release(tmp_path):
    wheel, release = _public_wheel(tmp_path)
    proof = verify_public_wheel(wheel, "1.2.3", release)
    assert proof["requires_dist"] == []
    assert proof["sha256"] == release["urls"][0]["digests"]["sha256"]
    with pytest.raises(ValueError, match="release identity"):
        verify_public_wheel(wheel, "1.2.4", release)
    wheel.write_bytes(wheel.read_bytes() + b"changed")
    with pytest.raises(ValueError, match="differs from the public release"):
        verify_public_wheel(wheel, "1.2.3", release)


def test_public_wheel_evidence_rejects_core_dependencies(tmp_path):
    wheel, release = _public_wheel(tmp_path, dependencies="Requires-Dist: unexpected-library\n")
    with pytest.raises(ValueError, match="runtime dependencies"):
        verify_public_wheel(wheel, "1.2.3", release)


def test_public_source_version_must_match_the_installed_release(tmp_path):
    (tmp_path / "pyproject.toml").write_text(
        '[project]\nname = "fullbleed"\nversion = "1.2.3"\n', encoding="utf-8")
    assert verify_source_version(tmp_path, "1.2.3") == "1.2.3"
    with pytest.raises(ValueError, match="source version"):
        verify_source_version(tmp_path, "1.2.4")
