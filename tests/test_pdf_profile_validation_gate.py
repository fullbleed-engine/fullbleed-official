import importlib.util
from pathlib import Path
import sys

import pytest

SPEC = importlib.util.spec_from_file_location(
    "profile_harness", Path(__file__).resolve().parents[1] / "tools/validate_pdf_profiles.py"
)
harness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(harness)


def validator(tmp_path, body):
    script = tmp_path / "validator.py"
    script.write_text(body, encoding="utf-8")
    # POSIX quoting is the command-template grammar on every host.
    return f'"{Path(sys.executable).as_posix()}" "{script.as_posix()}" {{pdf}} {{report}}'


def test_validator_paths_are_single_arguments_and_report_is_retained(tmp_path):
    pdf = tmp_path / "spaced input.pdf"
    pdf.write_bytes(b"fixture")
    command = validator(tmp_path, "import pathlib, sys\nassert pathlib.Path(sys.argv[1]).read_bytes() == b'fixture'\npathlib.Path(sys.argv[2]).write_text('{\"valid\":true}')\nprint('separate log')\n")
    result = harness.validate_with_pdfvt_command("pdfvt1", pdf, tmp_path, command)
    assert result["status"] == "passed"
    assert Path(result["report"]).read_text() == '{"valid":true}'
    assert "separate log" in Path(result["stdout"]).read_text()
    assert result["pdf_sha256"] == harness.sha256_file(pdf)


@pytest.mark.parametrize("command,reason", [
    (None, "dedicated_pdfvt_validator_not_configured"),
    ("echo success", "validator_command_requires_pdf_placeholder"),
    ("missing-dedicated-validator {pdf}", "validator_launch_failed"),
])
def test_unavailable_validator_never_passes(tmp_path, command, reason):
    pdf = tmp_path / "input.pdf"
    pdf.write_bytes(b"fixture")
    result = harness.validate_with_pdfvt_command("pdfvt1", pdf, tmp_path, command)
    assert result["status"] != "passed"
    assert result["reason"] == reason


@pytest.mark.parametrize("exit_code", [0, 2, 127])
def test_false_success_or_tool_error_fails_negative_controls(tmp_path, exit_code):
    pdf = tmp_path / "input.pdf"
    pdf.write_bytes(b'%PDF-1.6 /DPartRootNode 1 0 R pdfvtid:GTS_PDFVTModDate="2026-09-30')
    command = validator(tmp_path, f"raise SystemExit({exit_code})\n")
    result = harness.validate_pdfvt_negative_controls(pdf, tmp_path, command)
    assert result["status"] == "failed"
    assert len(result["controls"]) == 3
    assert all(r["status"] == "failed" for r in result["controls"].values())


def test_validator_version_requires_identifying_output(tmp_path):
    assert harness.pdfvt_validator_version(None, tmp_path)["status"] == "skipped"
    command = validator(tmp_path, "print('test validator version 1, PDF/VT-1 profile')\n")
    assert harness.pdfvt_validator_version(command, tmp_path)["status"] == "passed"


def test_verapdf_repairs_an_incomplete_extracted_cache(tmp_path):
    import io
    import zipfile
    pack, installer = io.BytesIO(), io.BytesIO()
    with zipfile.ZipFile(pack, "w") as archive:
        archive.writestr("org/verapdf/apps/GreenfieldCliWrapper.class", b"test class")
    with zipfile.ZipFile(installer, "w") as archive:
        archive.writestr("resources/packs/pack-veraPDF CLI", pack.getvalue())
    with zipfile.ZipFile(tmp_path / "verapdf-installer.zip", "w") as archive:
        archive.writestr("version/verapdf-installer.jar", installer.getvalue())
    cli_dir = harness.download_verapdf_classpath(tmp_path)
    entrypoint = cli_dir / "org/verapdf/apps/GreenfieldCliWrapper.class"
    entrypoint.unlink()
    assert harness.download_verapdf_classpath(tmp_path) == cli_dir
    assert entrypoint.read_bytes() == b"test class"
