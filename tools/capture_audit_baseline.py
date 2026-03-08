from __future__ import annotations

import argparse
import json
import shutil
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import fullbleed


REPO_ROOT = Path(__file__).resolve().parents[1]


def _now_iso_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=False), encoding="utf-8")


def _write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _resolve_path(path_text: str) -> Path:
    p = Path(path_text)
    if not p.is_absolute():
        p = REPO_ROOT / p
    return p.resolve()


def _a11y_nonpass_rule_ids(report: dict[str, Any]) -> list[str]:
    findings = report.get("findings") or []
    out = {
        str(row.get("rule_id", "")).strip()
        for row in findings
        if str(row.get("verdict", "")).strip() != "pass" and str(row.get("rule_id", "")).strip()
    }
    return sorted(out)


def _pmr_nonpass_audit_ids(report: dict[str, Any]) -> list[str]:
    audits = report.get("audits") or []
    out = {
        str(row.get("audit_id", "")).strip()
        for row in audits
        if str(row.get("verdict", "")).strip() != "pass" and str(row.get("audit_id", "")).strip()
    }
    return sorted(out)


def _markdown_table(rows: list[dict[str, Any]]) -> str:
    header = (
        "| Document | A11y Gate | A11y Nonpass | PMR Gate | PMR Score | PMR Band |\n"
        "|---|---:|---:|---:|---:|---|"
    )
    lines = [header]
    for row in rows:
        lines.append(
            "| {doc_id} | {a11y_gate_ok} | {a11y_nonpass_count} | {pmr_gate_ok} | {pmr_score} | {pmr_band} |".format(
                doc_id=row["doc_id"],
                a11y_gate_ok=str(bool(row["a11y_gate_ok"])).lower(),
                a11y_nonpass_count=row["a11y_nonpass_count"],
                pmr_gate_ok=str(bool(row["pmr_gate_ok"])).lower(),
                pmr_score=row["pmr_score"],
                pmr_band=row["pmr_band"],
            )
        )
    return "\n".join(lines)


def _counter_delta(
    current: dict[str, int],
    previous: dict[str, int],
) -> dict[str, dict[str, int]]:
    keys = sorted(set(current) | set(previous))
    out: dict[str, dict[str, int]] = {}
    for key in keys:
        cur = int(current.get(key, 0))
        prev = int(previous.get(key, 0))
        out[key] = {
            "previous": prev,
            "current": cur,
            "delta": cur - prev,
        }
    return out


def capture_baseline(
    manifest_path: Path,
    out_dir: Path,
    profile_override: str | None = None,
    mode_override: str | None = None,
    compare_summary_path: Path | None = None,
) -> dict[str, Any]:
    manifest = _load_json(manifest_path)
    docs = manifest.get("documents")
    if not isinstance(docs, list) or not docs:
        raise ValueError("manifest must include a non-empty 'documents' list")

    profile = str(profile_override or manifest.get("profile") or "cav")
    mode = str(mode_override or manifest.get("mode") or "error")

    out_dir.mkdir(parents=True, exist_ok=True)
    engine = fullbleed.PdfEngine()

    rows: list[dict[str, Any]] = []
    a11y_counter: Counter[str] = Counter()
    pmr_counter: Counter[str] = Counter()
    errors: list[dict[str, Any]] = []

    for item in docs:
        doc_id = str(item.get("id") or "").strip()
        if not doc_id:
            raise ValueError("every document item requires a non-empty 'id'")
        html_path = _resolve_path(str(item.get("html_path") or ""))
        css_path = _resolve_path(str(item.get("css_path") or ""))
        if not html_path.exists():
            raise FileNotFoundError(f"missing html_path for {doc_id}: {html_path}")
        if not css_path.exists():
            raise FileNotFoundError(f"missing css_path for {doc_id}: {css_path}")

        doc_out = out_dir / doc_id
        doc_out.mkdir(parents=True, exist_ok=True)

        try:
            a11y = dict(
                engine.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile=profile,
                    mode=mode,
                )
            )
            pmr = dict(
                engine.verify_paged_media_rank_artifacts(
                    str(html_path),
                    str(css_path),
                    profile=profile,
                    mode=mode,
                )
            )
        except Exception as exc:  # pragma: no cover
            errors.append(
                {
                    "doc_id": doc_id,
                    "error_type": type(exc).__name__,
                    "error": str(exc),
                }
            )
            continue

        html_copy = doc_out / f"{doc_id}.html"
        css_copy = doc_out / f"{doc_id}.css"
        shutil.copy2(html_path, html_copy)
        shutil.copy2(css_path, css_copy)

        a11y_out = doc_out / f"{doc_id}_a11y_verify_engine.json"
        pmr_out = doc_out / f"{doc_id}_pmr_engine.json"
        _write_json(a11y_out, a11y)
        _write_json(pmr_out, pmr)

        a11y_ids = _a11y_nonpass_rule_ids(a11y)
        pmr_ids = _pmr_nonpass_audit_ids(pmr)
        for rid in a11y_ids:
            a11y_counter[rid] += 1
        for aid in pmr_ids:
            pmr_counter[aid] += 1

        rank = pmr.get("rank") or {}
        score_block = pmr.get("score") or {}
        pmr_score = rank.get("score")
        if pmr_score is None:
            pmr_score = score_block.get("value")
        pmr_band = rank.get("band")
        if pmr_band is None:
            pmr_band = score_block.get("band")

        row = {
            "doc_id": doc_id,
            "html_path": str(html_path),
            "css_path": str(css_path),
            "baseline_html_path": str(html_copy),
            "baseline_css_path": str(css_copy),
            "baseline_a11y_path": str(a11y_out),
            "baseline_pmr_path": str(pmr_out),
            "a11y_gate_ok": bool((a11y.get("gate") or {}).get("ok")),
            "a11y_nonpass_count": len(a11y_ids),
            "a11y_nonpass_rule_ids": a11y_ids,
            "pmr_gate_ok": bool((pmr.get("gate") or {}).get("ok")),
            "pmr_score": pmr_score,
            "pmr_band": pmr_band,
            "pmr_nonpass_count": len(pmr_ids),
            "pmr_nonpass_audit_ids": pmr_ids,
        }
        rows.append(row)

    rows.sort(key=lambda r: r["doc_id"])
    summary = {
        "schema": "fullbleed.audit_baseline_capture.v1",
        "schema_version": 1,
        "generated_at": _now_iso_utc(),
        "repo_root": str(REPO_ROOT),
        "manifest_path": str(manifest_path),
        "profile": profile,
        "mode": mode,
        "document_count": len(rows),
        "documents": rows,
        "aggregate": {
            "a11y_rule_nonpass_frequency": dict(sorted(a11y_counter.items())),
            "pmr_audit_nonpass_frequency": dict(sorted(pmr_counter.items())),
            "a11y_gate_pass_count": sum(1 for r in rows if r["a11y_gate_ok"]),
            "pmr_gate_pass_count": sum(1 for r in rows if r["pmr_gate_ok"]),
        },
        "errors": errors,
    }

    compare_payload: dict[str, Any] | None = None
    if compare_summary_path:
        prev = _load_json(compare_summary_path)
        prev_agg = prev.get("aggregate") or {}
        cur_agg = summary.get("aggregate") or {}
        compare_payload = {
            "previous_summary_path": str(compare_summary_path),
            "a11y_rule_nonpass_frequency_delta": _counter_delta(
                dict(cur_agg.get("a11y_rule_nonpass_frequency") or {}),
                dict(prev_agg.get("a11y_rule_nonpass_frequency") or {}),
            ),
            "pmr_audit_nonpass_frequency_delta": _counter_delta(
                dict(cur_agg.get("pmr_audit_nonpass_frequency") or {}),
                dict(prev_agg.get("pmr_audit_nonpass_frequency") or {}),
            ),
            "a11y_gate_pass_count_delta": int(cur_agg.get("a11y_gate_pass_count") or 0)
            - int(prev_agg.get("a11y_gate_pass_count") or 0),
            "pmr_gate_pass_count_delta": int(cur_agg.get("pmr_gate_pass_count") or 0)
            - int(prev_agg.get("pmr_gate_pass_count") or 0),
        }
        summary["compare"] = compare_payload

    _write_json(out_dir / "baseline_summary.json", summary)

    lines = [
        "# Audit Baseline Capture",
        "",
        f"- Generated: `{summary['generated_at']}`",
        f"- Manifest: `{manifest_path}`",
        f"- Profile: `{profile}`",
        f"- Mode: `{mode}`",
        f"- Captured docs: `{len(rows)}`",
        "",
        _markdown_table(rows),
        "",
        "## Aggregate A11y Nonpass Rule Frequency",
        "",
        "```json",
        json.dumps(summary["aggregate"]["a11y_rule_nonpass_frequency"], indent=2),
        "```",
        "",
        "## Aggregate PMR Nonpass Audit Frequency",
        "",
        "```json",
        json.dumps(summary["aggregate"]["pmr_audit_nonpass_frequency"], indent=2),
        "```",
        "",
    ]
    if errors:
        lines.extend(
            [
                "## Errors",
                "",
                "```json",
                json.dumps(errors, indent=2),
                "```",
                "",
            ]
        )
    if compare_payload:
        lines.extend(
            [
                "## Comparison",
                "",
                f"- Previous summary: `{compare_payload['previous_summary_path']}`",
                "",
                "### A11y Rule Frequency Delta",
                "",
                "```json",
                json.dumps(compare_payload["a11y_rule_nonpass_frequency_delta"], indent=2),
                "```",
                "",
                "### PMR Audit Frequency Delta",
                "",
                "```json",
                json.dumps(compare_payload["pmr_audit_nonpass_frequency_delta"], indent=2),
                "```",
                "",
            ]
        )
    _write_text(out_dir / "baseline_summary.md", "\n".join(lines))
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Capture deterministic B0 baseline artifacts for the third-party accessibility audit."
    )
    parser.add_argument(
        "--manifest",
        default="audit_baseline/audited_docs.v1.json",
        help="Path to baseline manifest JSON.",
    )
    parser.add_argument(
        "--out-dir",
        default="audit_baseline/third_party_2026_03",
        help="Directory to write baseline artifacts.",
    )
    parser.add_argument(
        "--profile",
        default=None,
        help="Override verifier/PMR profile (defaults to manifest profile).",
    )
    parser.add_argument(
        "--mode",
        default=None,
        help="Override verifier/PMR mode (defaults to manifest mode).",
    )
    parser.add_argument(
        "--compare-summary",
        default=None,
        help="Optional previous baseline_summary.json path to emit rule-family deltas.",
    )
    args = parser.parse_args(argv)

    manifest_path = _resolve_path(args.manifest)
    out_dir = _resolve_path(args.out_dir)
    compare_summary_path = _resolve_path(args.compare_summary) if args.compare_summary else None
    summary = capture_baseline(
        manifest_path=manifest_path,
        out_dir=out_dir,
        profile_override=args.profile,
        mode_override=args.mode,
        compare_summary_path=compare_summary_path,
    )
    print(
        json.dumps(
            {
                "ok": not bool(summary.get("errors")),
                "document_count": summary.get("document_count"),
                "out_dir": str(out_dir),
                "summary_path": str(out_dir / "baseline_summary.json"),
            },
            indent=2,
        )
    )
    return 0 if not summary.get("errors") else 2


if __name__ == "__main__":
    raise SystemExit(main())
