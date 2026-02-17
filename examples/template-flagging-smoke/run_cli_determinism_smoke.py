from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import fullbleed


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
OUT.mkdir(parents=True, exist_ok=True)


NON_HTML = OUT / "cli_det_noncompose_input.html"
NON_CSS = OUT / "cli_det_noncompose_input.css"

COMPOSE_TEMPLATE_PDF = OUT / "cli_det_template_blue.pdf"
COMPOSE_HTML = OUT / "cli_det_compose_input.html"
COMPOSE_CSS = OUT / "cli_det_compose_input.css"
COMPOSE_BINDING_JSON = OUT / "cli_det_compose_binding.json"
COMPOSE_TEMPLATES_JSON = OUT / "cli_det_compose_templates.json"

REPORT_JSON = OUT / "cli_determinism_smoke_report.json"


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _stable_json_hash(payload: dict[str, Any] | list[Any]) -> str:
    normalized = json.dumps(
        payload,
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


def _artifact_set_sha256(pdf_sha256: str | None, image_sha256: list[str]) -> str:
    payload = {
        "schema": "fullbleed.artifact_digest.v1",
        "pdf_sha256": pdf_sha256,
        "image_sha256": image_sha256,
    }
    return _stable_json_hash(payload)


def _write_json(path: Path, payload: dict[str, Any] | list[Any]) -> None:
    path.write_text(json.dumps(payload, ensure_ascii=True, indent=2), encoding="utf-8")


def _run_cli_render(args: list[str], *, threads: int) -> dict[str, Any]:
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = str(threads)
    proc = subprocess.run(args, capture_output=True, text=True, check=False, env=env)
    if proc.returncode != 0:
        raise RuntimeError(
            "cli render failed "
            f"rc={proc.returncode} stdout={proc.stdout.strip()} stderr={proc.stderr.strip()}"
        )
    try:
        payload = json.loads(proc.stdout.strip())
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"cli returned non-json output: {proc.stdout!r}") from exc
    if not payload.get("ok", False):
        raise RuntimeError(f"cli payload not ok: {payload}")
    return payload


def _collect_signature(
    outputs: dict[str, Any], expected_mode: str, expected_pdf_path: Path
) -> dict[str, Any]:
    image_mode = outputs.get("image_mode")
    if image_mode != expected_mode:
        raise RuntimeError(f"expected image_mode={expected_mode!r}, got {image_mode!r}")

    output_pdf = outputs.get("pdf") or outputs.get("output_pdf")
    pdf_path = Path(output_pdf) if output_pdf else expected_pdf_path
    if not pdf_path.exists():
        raise RuntimeError(f"output_pdf does not exist: {pdf_path}")

    image_paths = outputs.get("image_paths") or []
    if not image_paths:
        raise RuntimeError("expected non-empty image_paths")
    resolved_images = [Path(p) for p in image_paths]
    for p in resolved_images:
        if not p.exists():
            raise RuntimeError(f"image artifact missing: {p}")

    pdf_sha256 = _sha256_file(pdf_path)
    image_sha256 = [_sha256_file(p) for p in resolved_images]
    artifact_sha256 = _artifact_set_sha256(pdf_sha256, image_sha256)

    output_pdf_sha256 = outputs.get("sha256")
    if output_pdf_sha256 and output_pdf_sha256 != pdf_sha256:
        raise RuntimeError(
            f"outputs.sha256 mismatch: outputs={output_pdf_sha256} computed={pdf_sha256}"
        )
    output_image_sha256 = outputs.get("image_sha256")
    if output_image_sha256 and output_image_sha256 != image_sha256:
        raise RuntimeError(
            f"outputs.image_sha256 mismatch: outputs={output_image_sha256} computed={image_sha256}"
        )
    output_artifact_sha256 = outputs.get("artifact_sha256")
    if output_artifact_sha256 and output_artifact_sha256 != artifact_sha256:
        raise RuntimeError(
            f"outputs.artifact_sha256 mismatch: outputs={output_artifact_sha256} computed={artifact_sha256}"
        )
    deterministic_mode = outputs.get("deterministic_hash_mode")
    if deterministic_mode and deterministic_mode != "artifact_set_v1":
        raise RuntimeError(
            f"expected deterministic_hash_mode=artifact_set_v1, got {deterministic_mode!r}"
        )
    deterministic_value = outputs.get("deterministic_hash_sha256")
    if deterministic_value and deterministic_value != artifact_sha256:
        raise RuntimeError(
            f"outputs.deterministic_hash_sha256 mismatch: outputs={deterministic_value} computed={artifact_sha256}"
        )

    return {
        "output_pdf": str(pdf_path),
        "pdf_sha256": pdf_sha256,
        "image_paths": [str(p) for p in resolved_images],
        "image_sha256": image_sha256,
        "artifact_sha256": artifact_sha256,
        "image_mode": image_mode,
        "image_count": len(resolved_images),
        "deterministic_hash_mode": deterministic_mode,
        "deterministic_hash_output": deterministic_value,
    }


def _assert_signatures_equal(label: str, a: dict[str, Any], b: dict[str, Any]) -> None:
    if a["pdf_sha256"] != b["pdf_sha256"]:
        raise RuntimeError(
            f"{label} pdf hash mismatch: run_a={a['pdf_sha256']} run_b={b['pdf_sha256']}"
        )
    if a["image_sha256"] != b["image_sha256"]:
        raise RuntimeError(
            f"{label} image hash mismatch: run_a={a['image_sha256']} run_b={b['image_sha256']}"
        )
    if a["artifact_sha256"] != b["artifact_sha256"]:
        raise RuntimeError(
            f"{label} artifact hash mismatch: run_a={a['artifact_sha256']} run_b={b['artifact_sha256']}"
        )


def _write_non_compose_inputs() -> None:
    html = """
<!doctype html>
<html><body>
<section class="page"><h1>Determinism A</h1><p>alpha beta gamma</p></section>
<section class="page"><h1>Determinism B</h1><p>delta epsilon zeta</p></section>
</body></html>
""".strip()
    css = """
@page { size: 8.5in 11in; margin: 0.5in; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; }
.page:not(:last-child) { break-after: page; }
h1 { margin: 0 0 8pt 0; font-size: 18pt; }
p { margin: 0; font-size: 11pt; }
""".strip()
    NON_HTML.write_text(html, encoding="utf-8")
    NON_CSS.write_text(css, encoding="utf-8")


def _build_compose_template(path: Path) -> None:
    html = "<!doctype html><html><body><section class='tpl'><p>TEMPLATE</p></section></body></html>"
    css = """
@page { size: 8.5in 11in; margin: 0; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; }
.tpl { width: 8.5in; height: 11in; background: rgb(0,0,255); box-sizing: border-box; padding: 12pt; }
p { margin: 0; color: #fff; font-size: 10pt; }
""".strip()
    engine = fullbleed.PdfEngine(page_width="8.5in", page_height="11in", margin="0pt")
    engine.render_pdf_to_file(html, css, str(path))


def _write_compose_inputs() -> None:
    html = """
<!doctype html>
<html><body>
<section class="p"><div data-fb="fb.feature.blue=1"></div><p>Compose A</p></section>
<section class="p"><div data-fb="fb.feature.blue=1"></div><p>Compose B</p></section>
</body></html>
""".strip()
    css = """
@page { size: 8.5in 11in; margin: 0.5in; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; color: #111; }
.p:not(:last-child) { break-after: page; }
p { margin: 0; }
""".strip()
    binding = {
        "default_template_id": "tpl-blue",
        "by_feature": {"blue": "tpl-blue"},
        "feature_prefix": "fb.feature.",
    }
    templates = [{"template_id": "tpl-blue", "pdf_path": str(COMPOSE_TEMPLATE_PDF)}]

    COMPOSE_HTML.write_text(html, encoding="utf-8")
    COMPOSE_CSS.write_text(css, encoding="utf-8")
    _write_json(COMPOSE_BINDING_JSON, binding)
    _write_json(COMPOSE_TEMPLATES_JSON, templates)


def _run_non_compose_case(run_label: str, *, threads: int) -> dict[str, Any]:
    out_pdf = OUT / f"cli_det_noncompose_{run_label}.pdf"
    out_img_dir = OUT / f"cli_det_noncompose_{run_label}_images"
    out_hash = OUT / f"cli_det_noncompose_{run_label}.sha256"
    out_img_dir.mkdir(parents=True, exist_ok=True)
    args = [
        sys.executable,
        "-m",
        "fullbleed_cli.cli",
        "--json",
        "render",
        "--html",
        str(NON_HTML),
        "--css",
        str(NON_CSS),
        "--out",
        str(out_pdf),
        "--emit-image",
        str(out_img_dir),
        "--image-dpi",
        "120",
        "--deterministic-hash",
        str(out_hash),
    ]
    payload = _run_cli_render(args, threads=threads)
    outputs = payload.get("outputs") or {}
    sig = _collect_signature(outputs, "overlay_document", out_pdf)
    hash_file_value = out_hash.read_text(encoding="utf-8").strip() if out_hash.exists() else None
    sig["deterministic_hash_file"] = hash_file_value
    expected = sig["artifact_sha256"] or sig["pdf_sha256"]
    if hash_file_value and hash_file_value != expected:
        raise RuntimeError(
            f"non-compose deterministic hash file mismatch: file={hash_file_value} computed={expected}"
        )
    return {
        "threads": threads,
        "render": payload,
        "signature": sig,
    }


def _run_compose_case(run_label: str, *, threads: int) -> dict[str, Any]:
    out_pdf = OUT / f"cli_det_compose_{run_label}.pdf"
    out_img_dir = OUT / f"cli_det_compose_{run_label}_images"
    out_hash = OUT / f"cli_det_compose_{run_label}.sha256"
    out_img_dir.mkdir(parents=True, exist_ok=True)
    args = [
        sys.executable,
        "-m",
        "fullbleed_cli.cli",
        "--json",
        "render",
        "--html",
        str(COMPOSE_HTML),
        "--css",
        str(COMPOSE_CSS),
        "--template-binding",
        str(COMPOSE_BINDING_JSON),
        "--templates",
        str(COMPOSE_TEMPLATES_JSON),
        "--out",
        str(out_pdf),
        "--emit-image",
        str(out_img_dir),
        "--image-dpi",
        "120",
        "--deterministic-hash",
        str(out_hash),
    ]
    payload = _run_cli_render(args, threads=threads)
    outputs = payload.get("outputs") or {}
    sig = _collect_signature(outputs, "composed_pdf", out_pdf)
    hash_file_value = out_hash.read_text(encoding="utf-8").strip() if out_hash.exists() else None
    sig["deterministic_hash_file"] = hash_file_value
    expected = sig["artifact_sha256"] or sig["pdf_sha256"]
    if hash_file_value and hash_file_value != expected:
        raise RuntimeError(
            f"compose deterministic hash file mismatch: file={hash_file_value} computed={expected}"
        )
    return {
        "threads": threads,
        "render": payload,
        "signature": sig,
    }


def main() -> None:
    _write_non_compose_inputs()
    _build_compose_template(COMPOSE_TEMPLATE_PDF)
    _write_compose_inputs()

    non_run_a = _run_non_compose_case("run_a_t1", threads=1)
    non_run_b = _run_non_compose_case("run_b_t4", threads=4)
    _assert_signatures_equal(
        "non_compose", non_run_a["signature"], non_run_b["signature"]
    )

    compose_run_a = _run_compose_case("run_a_t1", threads=1)
    compose_run_b = _run_compose_case("run_b_t4", threads=4)
    _assert_signatures_equal(
        "compose", compose_run_a["signature"], compose_run_b["signature"]
    )

    report = {
        "schema": "fullbleed.template_flagging_cli_determinism_smoke.v1",
        "ok": True,
        "checks": {
            "cross_process_non_compose": {
                "ok": True,
                "run_a": non_run_a["signature"],
                "run_b": non_run_b["signature"],
            },
            "cross_process_compose": {
                "ok": True,
                "run_a": compose_run_a["signature"],
                "run_b": compose_run_b["signature"],
            },
        },
        "thread_profiles": {"run_a": 1, "run_b": 4},
        "paths": {
            "non_compose_html": str(NON_HTML),
            "non_compose_css": str(NON_CSS),
            "compose_html": str(COMPOSE_HTML),
            "compose_css": str(COMPOSE_CSS),
            "compose_template_pdf": str(COMPOSE_TEMPLATE_PDF),
        },
    }
    REPORT_JSON.write_text(json.dumps(report, ensure_ascii=True, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))


if __name__ == "__main__":
    main()
