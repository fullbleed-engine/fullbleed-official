from __future__ import annotations

import copy
import json
import os
from types import SimpleNamespace

import pytest

from fullbleed_cli import cli


@pytest.fixture
def current(monkeypatch):
    record = {
        "schema": "fullbleed.repro_record.v1",
        "input_fingerprint_sha256": "a" * 64,
        "output_pdf_sha256": "b" * 64,
        "assets_lock": None,
    }
    monkeypatch.setattr(cli, "_build_repro_record", lambda *args: copy.deepcopy(record))
    return record


def run_check(path, record_path=None):
    args = SimpleNamespace(repro_check=str(path), repro_record=record_path)
    return cli._run_repro_record_or_check(args, {}, "", "", "b" * 64, 123)


@pytest.mark.parametrize("record", [
    {}, [], None, False, 123, "baseline",
    {"schema": "fullbleed.repro_record.v1"},
    {"schema": "other", "input_fingerprint_sha256": "a" * 64, "output_pdf_sha256": "b" * 64},
])
def test_malformed_baseline_never_passes(tmp_path, current, record):
    path = tmp_path / "baseline.json"
    before = json.dumps(record).encode()
    path.write_bytes(before)
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_RECORD_INVALID"]
    assert path.read_bytes() == before


@pytest.mark.parametrize("field", ["schema", "input_fingerprint_sha256", "output_pdf_sha256"])
def test_each_required_field_is_checked(tmp_path, current, field):
    record = dict(current)
    del record[field]
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(record), encoding="utf-8")
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_RECORD_INVALID"]
    assert field in failures[0]["message"]


@pytest.mark.parametrize(("field", "value"), [
    ("input_fingerprint_sha256", ""), ("output_pdf_sha256", None),
    ("output_pdf_sha256", []), ("output_pdf_sha256", 3),
    ("output_pdf_sha256", "not-a-hash"), ("output_pdf_sha256", "z" * 64),
    ("input_manifest_sha256", "a" * 63), ("output_bytes_written", True),
    ("output_bytes_written", -1), ("cli_version", 252),
    ("assets_lock", []), ("assets_lock", {}),
    ("assets_lock", {"sha256": ""}), ("assets_lock", {"sha256": "c" * 64, "path": []}),
])
def test_malformed_known_fields_are_reported(tmp_path, current, field, value):
    record = dict(current, **{field: value})
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(record), encoding="utf-8")
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_RECORD_INVALID"]
    assert field in failures[0]["message"]


@pytest.mark.parametrize("raw", [b"{", b"\xff\xfe", b""])
def test_unreadable_json_has_a_structured_record_error(tmp_path, current, raw):
    path = tmp_path / "baseline.json"
    path.write_bytes(raw)
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_RECORD_INVALID"]


def test_missing_baseline_does_not_pass_or_create_it(tmp_path, current):
    path = tmp_path / "missing.json"
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_RECORD_NOT_FOUND"]
    assert not path.exists()


def test_valid_minimum_record_and_future_fields_remain_compatible(tmp_path, current):
    record = {key: current[key] for key in cli.SCHEMA_DEFS["fullbleed.repro_record.v1"]["required"]}
    record["future_metadata"] = {"detail": "unknown fields remain compatible"}
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(record), encoding="utf-8")
    _, failures = run_check(path)
    assert failures == []


def test_input_and_pdf_changes_both_fail(tmp_path, current):
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(dict(current, input_fingerprint_sha256="c" * 64,
                                    output_pdf_sha256="d" * 64)), encoding="utf-8")
    _, failures = run_check(path)
    assert {f["code"] for f in failures} == {"REPRO_INPUT_DRIFT", "REPRO_HASH_MISMATCH"}


@pytest.mark.parametrize(("expected", "observed"), [
    ({"sha256": "c" * 64}, None),
    (None, {"sha256": "c" * 64}),
    ({"sha256": "c" * 64}, {"sha256": "d" * 64}),
])
def test_lockfile_content_and_presence_changes_fail(tmp_path, current, expected, observed):
    path = tmp_path / "baseline.json"
    path.write_text(json.dumps(dict(current, assets_lock=expected)), encoding="utf-8")
    current["assets_lock"] = observed
    _, failures = run_check(path)
    assert [f["code"] for f in failures] == ["REPRO_LOCK_MISMATCH"]


@pytest.mark.parametrize("alias", ["same", "relative", "hardlink"])
def test_check_cannot_overwrite_its_own_baseline(tmp_path, monkeypatch, current, alias):
    path = tmp_path / "baseline.json"
    before = json.dumps(dict(current, output_pdf_sha256="c" * 64)).encode()
    path.write_bytes(before)
    if alias == "hardlink":
        output = tmp_path / "alias.json"
        os.link(path, output)
    elif alias == "relative":
        monkeypatch.chdir(tmp_path)
        output = "./baseline.json"
    else:
        output = path
    _, failures = run_check(path, str(output))
    assert [f["code"] for f in failures] == ["REPRO_RECORD_CONFLICT"]
    assert path.read_bytes() == before


def test_separate_candidate_record_is_written_without_replacing_baseline(tmp_path, current):
    path = tmp_path / "baseline.json"
    candidate = tmp_path / "candidate.json"
    before = json.dumps(dict(current, output_pdf_sha256="c" * 64)).encode()
    path.write_bytes(before)
    _, failures = run_check(path, str(candidate))
    assert [f["code"] for f in failures] == ["REPRO_HASH_MISMATCH"]
    assert path.read_bytes() == before
    assert json.loads(candidate.read_text()) == current


def test_record_errors_have_actionable_diagnostics():
    failures = [{"code": "REPRO_RECORD_INVALID"}, {"code": "REPRO_RECORD_CONFLICT"}]
    cli._enrich_repro_failures(failures)
    assert all(f["recommended_actions"] and f["relevant_commands"] for f in failures)


def test_record_schema_is_discoverable(capsys):
    name = cli._infer_schema_from_argv(["--schema", "repro-record"])
    assert name == "fullbleed.repro_record.v1"
    cli._emit_schema(name)
    result = json.loads(capsys.readouterr().out)
    assert result["definition"] == cli.SCHEMA_DEFS[name]
