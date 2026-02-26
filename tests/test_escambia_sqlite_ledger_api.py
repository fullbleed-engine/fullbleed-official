from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

pytest.importorskip("fastapi")
from fastapi.testclient import TestClient


REPO_ROOT = Path(__file__).resolve().parents[1]
ESCAMBIA_DIR = REPO_ROOT / "_escambia"
if str(ESCAMBIA_DIR) not in sys.path:
    sys.path.insert(0, str(ESCAMBIA_DIR))

import sqlite_ledger as ledger
import sqlite_ledger_api as api


def _seed_minimal_db(db_path: Path) -> None:
    conn = ledger.connect(db_path)
    ledger.init_db(conn)
    conn.execute(
        """
        INSERT INTO cav_runs(
          exemplar_id, exemplar_root, run_report_path, run_report_sha256, run_report_generated_at,
          ui_kit_used, actual_profile_id, raw_json, ingested_at, a11y_ok, pmr_ok, natural_a11y_ok, pmr_score
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "_escambia/sample_exemplar",
            str((REPO_ROOT / "_escambia" / "sample_exemplar").resolve()),
            str((REPO_ROOT / "_escambia" / "sample_exemplar" / "output" / "sample_run_report.json").resolve()),
            "deadbeef",
            "2026-02-25T00:00:00+00:00",
            "CourtMotionFormCavKit",
            "fl.escambia.sample.v1",
            "{}",
            "2026-02-25T00:00:01+00:00",
            1,
            1,
            1,
            100.0,
        ),
    )
    cav_run_id = int(conn.execute("SELECT id FROM cav_runs LIMIT 1").fetchone()[0])
    conn.execute(
        """
        INSERT INTO cav_latest(exemplar_id, cav_run_id, run_report_sha256, updated_at)
        VALUES (?, ?, ?, ?)
        """,
        ("_escambia/sample_exemplar", cav_run_id, "deadbeef", "2026-02-25T00:00:02+00:00"),
    )
    conn.execute(
        """
        INSERT INTO doccenter_documents(
          doccenter_id, first_url, last_url, last_final_url, last_status, last_http_status, last_content_type,
          last_sha256, last_file_path, last_error, success_pdf_count, fail_count, duplicate_content_count,
          first_seen_at, last_fetched_at, raw_last_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            728,
            "https://example.test/DocumentCenter/View/728",
            "https://example.test/DocumentCenter/View/728",
            "https://example.test/files/728.pdf",
            "ok_pdf",
            200,
            "application/pdf",
            "beadfeed",
            str((REPO_ROOT / "_escambia" / "sources" / "doccenter_728.pdf").resolve()),
            None,
            1,
            0,
            0,
            "2026-02-25T00:00:00+00:00",
            "2026-02-25T00:00:03+00:00",
            "{}",
        ),
    )
    conn.execute(
        """
        INSERT INTO fetch_events(
          doccenter_id, requested_url, final_url, status, http_status, content_type, size_bytes,
          sha256, file_path, error, duplicate_of_sha256, requested_at, completed_at, raw_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            728,
            "https://example.test/DocumentCenter/View/728",
            "https://example.test/files/728.pdf",
            "ok_pdf",
            200,
            "application/pdf",
            1024,
            "beadfeed",
            str((REPO_ROOT / "_escambia" / "sources" / "doccenter_728.pdf").resolve()),
            None,
            None,
            "2026-02-25T00:00:00+00:00",
            "2026-02-25T00:00:04+00:00",
            "{}",
        ),
    )
    conn.commit()
    conn.close()


def test_escambia_ledger_api_lists_and_timestamp_aliases(tmp_path: Path) -> None:
    db_path = tmp_path / "escambia_api_test.db"
    _seed_minimal_db(db_path)

    client = TestClient(api.create_app(db_path=db_path))

    health = client.get("/health")
    assert health.status_code == 200
    assert health.json()["ok"] is True

    stats = client.get("/stats")
    assert stats.status_code == 200
    stats_payload = stats.json()
    assert stats_payload["cav_runs"] == 1
    assert stats_payload["cav_latest"] == 1
    assert stats_payload["doccenter_documents"] == 1
    assert stats_payload["fetch_events"] == 1

    latest = client.get("/cav/latest")
    assert latest.status_code == 200
    latest_payload = latest.json()
    assert latest_payload["total"] == 1
    latest_item = latest_payload["items"][0]
    assert latest_item["created_at"] == "2026-02-25T00:00:01+00:00"
    assert latest_item["updated_at"] == "2026-02-25T00:00:02+00:00"

    runs = client.get("/cav/runs", params={"exemplar_id": "_escambia/sample_exemplar"})
    assert runs.status_code == 200
    runs_payload = runs.json()
    assert runs_payload["total"] == 1
    run_item = runs_payload["items"][0]
    assert run_item["created_at"] == "2026-02-25T00:00:01+00:00"
    assert run_item["updated_at"] == "2026-02-25T00:00:01+00:00"

    docs = client.get("/doccenter/documents", params={"has_success": True})
    assert docs.status_code == 200
    docs_payload = docs.json()
    assert docs_payload["total"] == 1
    doc_item = docs_payload["items"][0]
    assert doc_item["created_at"] == "2026-02-25T00:00:00+00:00"
    assert doc_item["updated_at"] == "2026-02-25T00:00:03+00:00"

    events = client.get("/fetch/events", params={"doccenter_id": 728})
    assert events.status_code == 200
    events_payload = events.json()
    assert events_payload["total"] == 1
    evt = events_payload["items"][0]
    assert evt["created_at"] == "2026-02-25T00:00:00+00:00"
    assert evt["updated_at"] == "2026-02-25T00:00:04+00:00"


def test_escambia_ledger_api_ingest_endpoint(tmp_path: Path) -> None:
    db_path = tmp_path / "escambia_api_ingest.db"
    conn = ledger.connect(db_path)
    ledger.init_db(conn)
    conn.close()

    exemplar_root = tmp_path / "demo_exemplar"
    output_dir = exemplar_root / "output"
    output_dir.mkdir(parents=True, exist_ok=True)
    run_report_path = output_dir / "demo_exemplar_run_report.json"
    run_report_path.write_text(
        json.dumps(
            {
                "source_pdf_path": str(exemplar_root / "sources" / "demo_exemplar.pdf"),
                "actual_profile_id": "fl.escambia.demo.v1",
                "deliverables": {
                    "html_path": str(output_dir / "demo_exemplar.html"),
                    "css_path": str(output_dir / "demo_exemplar.css"),
                    "pdf_preview_path": str(output_dir / "demo_exemplar.pdf"),
                },
                "metrics": {
                    "engine_a11y_verify_ok": True,
                    "engine_pmr_ok": True,
                    "pdf_ua_seed_ok": True,
                    "engine_a11y_natural_pass_ok": True,
                    "engine_pmr_score": 100.0,
                },
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    client = TestClient(api.create_app(db_path=db_path))
    resp = client.post("/cav/ingest", json={"roots": [str(tmp_path)]})
    assert resp.status_code == 200
    payload = resp.json()
    assert payload["scanned"] == 1
    assert payload["ingested"] == 1
    assert payload["error_count"] == 0

    runs = client.get("/cav/runs")
    assert runs.status_code == 200
    assert runs.json()["total"] == 1
