#!/usr/bin/env python3
"""Generate and validate deterministic PDF profile specimens.

This is an operational conformance harness. It keeps the generated PDFs,
render logs, inspector output, external validator reports, and replay hashes
under one output directory so profile claims can be audited after the run.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib
import importlib.metadata
import io
import json
import os
import platform
import shlex
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PROFILES = [
    "pdfa1a",
    "pdfa1b",
    "pdfa2a",
    "pdfa2b",
    "pdfa2u",
    "pdfa3a",
    "pdfa3b",
    "pdfa3u",
    "pdfa4",
    "pdfa4e",
    "pdfa4f",
    "pdfua1",
    "pdfua2",
    "wtpdf1r",
    "wtpdf1a",
    "pdfx4",
    "pdfvt1",
]
OUTPUT_INTENT_PROFILES = {
    "pdfa1a",
    "pdfa1b",
    "pdfa2a",
    "pdfa2b",
    "pdfa2u",
    "pdfa3a",
    "pdfa3b",
    "pdfa3u",
    "pdfa4",
    "pdfa4e",
    "pdfa4f",
    "pdfx4",
    "pdfvt1",
}
EMBEDDED_FONT_PROFILES = {
    "pdfa1a",
    "pdfa1b",
    "pdfa2a",
    "pdfa2b",
    "pdfa2u",
    "pdfa3a",
    "pdfa3b",
    "pdfa3u",
    "pdfa4",
    "pdfa4e",
    "pdfa4f",
    "pdfx4",
    "pdfua1",
    "pdfua2",
    "pdfvt1",
    "wtpdf1r",
    "wtpdf1a",
}
VERAPDF_FLAVOURS = {
    "pdfa1a": "1a",
    "pdfa1b": "1b",
    "pdfa2a": "2a",
    "pdfa2b": "2b",
    "pdfa2u": "2u",
    "pdfa3a": "3a",
    "pdfa3b": "3b",
    "pdfa3u": "3u",
    "pdfa4": "4",
    "pdfa4e": "4e",
    "pdfa4f": "4f",
    "pdfua1": "ua1",
    "pdfua2": "ua2",
    "wtpdf1r": "wt1r",
    "wtpdf1a": "wt1a",
}
PDFX_PROFILES = {"pdfx4", "pdfvt1"}
TAGGED_STRUCTURE_PROFILES = {
    "pdfa1a",
    "pdfa2a",
    "pdfa3a",
    "pdfua1",
    "pdfua2",
    "wtpdf1r",
    "wtpdf1a",
}
VERAPDF_URL = "https://software.verapdf.org/releases/verapdf-installer.zip"
PDFVT_DPART_INSPECTION_KEYS = [
    "dpart_root_present",
    "dpart_present",
    "page_dpart_present",
    "pdfvt_dpart_root_node_valid",
    "pdfvt_dpart_parent_valid",
    "pdfvt_dpart_node_name_list_valid",
    "pdfvt_dpart_leaf_valid",
    "pdfvt_dpart_page_range_valid",
    "pdfvt_dpart_graph_valid",
    "pdfvt_mod_date_matches_xmp",
]


HTML_SPECIMEN = (
    "<html><head><title>Conformance specimen</title></head><body><main>"
    "<h1>Conformance specimen</h1>"
    "<p>This deterministic specimen exercises embedded text, Unicode omega "
    "\u03a9, document language, title metadata, and tagged structure where "
    "applicable.</p>"
    "</main></body></html>"
)
CSS_SPECIMEN = (
    "@page { size: letter; margin: 0.75in; } "
    "body { margin: 0; font-family: Inter; font-size: 12pt; "
    "line-height: 1.35; color: #111; } "
    "h1 { font-size: 18pt; margin: 0 0 12pt 0; } "
    "p { margin: 0; }"
)
PDFVT_MULTIPAGE_HTML_SPECIMEN = (
    "<html><head><title>PDF/VT multipage conformance specimen</title></head>"
    "<body><main>"
    "<section><h1>PDF/VT document part page one</h1>"
    "<p>This supplemental specimen proves that the PDF/VT DPart range starts "
    "on the first page.</p></section>"
    "<section style=\"page-break-before: always; break-before: page;\">"
    "<h1>PDF/VT document part page two</h1>"
    "<p>This page proves that the DPart range ends on the last page and that "
    "each page references the document part.</p></section>"
    "</main></body></html>"
)
PDFVT_MULTIPAGE_CSS_SPECIMEN = CSS_SPECIMEN + " section { margin: 0 0 18pt 0; }"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_capture(cmd: list[str], stdout_path: Path, stderr_path: Path, *, timeout: int = 300) -> int:
    with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
        proc = subprocess.run(cmd, cwd=REPO_ROOT, stdout=out, stderr=err, check=False, timeout=timeout)
    return proc.returncode


def decode_json_report(path: Path) -> object:
    raw = path.read_bytes()
    if raw[:2] in (b"\xff\xfe", b"\xfe\xff"):
        text = raw.decode("utf-16")
    else:
        text = raw.decode("utf-8-sig")
    return json.loads(text)


def default_font_path() -> Path:
    return REPO_ROOT / "python" / "fullbleed_assets" / "fonts" / "Inter-Variable.ttf"


def default_icc_candidates() -> list[Path]:
    env = os.environ.get("FULLBLEED_OUTPUT_INTENT_ICC")
    candidates: list[Path] = []
    if env:
        candidates.append(Path(env))
    candidates.extend(
        [
            Path(r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm"),
            Path("/System/Library/ColorSync/Profiles/sRGB Profile.icc"),
            Path("/usr/share/color/icc/colord/sRGB.icc"),
            Path("/usr/share/color/icc/sRGB.icc"),
        ]
    )
    return candidates


def resolve_icc(path_arg: str | None) -> Path:
    if path_arg:
        path = Path(path_arg)
        if path.exists():
            return path
        raise SystemExit(f"ICC profile not found: {path}")
    for candidate in default_icc_candidates():
        if candidate.exists():
            return candidate
    searched = ", ".join(str(p) for p in default_icc_candidates())
    raise SystemExit(
        "No sRGB ICC profile found. Set --icc or FULLBLEED_OUTPUT_INTENT_ICC. "
        f"Searched: {searched}"
    )


def render_command(
    profile: str,
    pdf_path: Path,
    font_path: Path,
    icc_path: Path,
    *,
    html: str = HTML_SPECIMEN,
    css: str = CSS_SPECIMEN,
    title: str | None = None,
) -> list[str]:
    cmd = [
        sys.executable,
        "-m",
        "fullbleed_cli",
        "--json",
        "render",
        "--html-str",
        html,
        "--css-str",
        css,
        "--asset",
        str(font_path),
        "--asset-kind",
        "font",
        "--asset-name",
        "Inter",
        "--pdf-profile",
        profile,
        "--document-lang",
        "en-US",
        "--document-title",
        title or f"FullBleed {profile} conformance specimen",
        "--out",
        str(pdf_path),
    ]
    if profile in OUTPUT_INTENT_PROFILES:
        cmd.extend(
            [
                "--output-intent-icc",
                str(icc_path),
                "--output-intent-identifier",
                "sRGB IEC61966-2.1",
                "--output-intent-info",
                "sRGB IEC61966-2.1",
                "--output-intent-components",
                "3",
            ]
        )
    if profile in PDFX_PROFILES:
        cmd.extend(["--timestamp", "2026-09-30T00:00:00Z"])
    return cmd


def render_profile(
    profile: str,
    out_dir: Path,
    font_path: Path,
    icc_path: Path,
    *,
    emit_observability: bool,
    html: str = HTML_SPECIMEN,
    css: str = CSS_SPECIMEN,
    title: str | None = None,
) -> dict[str, object]:
    pdf_path = out_dir / f"{profile}.pdf"
    cmd = render_command(profile, pdf_path, font_path, icc_path, html=html, css=css, title=title)
    if emit_observability:
        cmd[-2:-2] = [
            "--emit-jit",
            str(out_dir / f"{profile}.jit.jsonl"),
            "--emit-manifest",
            str(out_dir / f"{profile}.manifest.json"),
        ]
    stdout_path = out_dir / f"{profile}.render.stdout.json"
    stderr_path = out_dir / f"{profile}.render.stderr.txt"
    exit_code = run_capture(cmd, stdout_path, stderr_path)
    return {
        "exit": exit_code,
        "pdf": str(pdf_path),
        "stdout": str(stdout_path),
        "stderr": str(stderr_path),
        "size": pdf_path.stat().st_size if pdf_path.exists() else 0,
        "sha256": sha256_file(pdf_path) if pdf_path.exists() else None,
    }


def inspect_profile(profile: str, pdf_path: Path, out_dir: Path) -> dict[str, object]:
    report_path = out_dir / f"{profile}.inspect.json"
    err_path = out_dir / f"{profile}.inspect.stderr.txt"
    exit_code = run_capture(
        [sys.executable, "-m", "fullbleed_cli", "inspect", "pdf", str(pdf_path), "--json"],
        report_path,
        err_path,
    )
    report: dict[str, object] | None = None
    checks: list[str] = []
    if exit_code == 0:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        profile_report = report.get("profile", {})
        claims = set(profile_report.get("claims", []))
        if profile not in claims:
            checks.append(f"missing_claim:{profile}")
        if profile in OUTPUT_INTENT_PROFILES and not profile_report.get("output_intent_present"):
            checks.append("missing_output_intent")
        if not profile_report.get("metadata_present"):
            checks.append("missing_metadata")
        if profile in EMBEDDED_FONT_PROFILES and int(
            profile_report.get("embedded_font_count", 0)
        ) < 1:
            checks.append("missing_embedded_font")
        if profile == "pdfa4f" and not profile_report.get("embedded_files_present"):
            checks.append("missing_embedded_files")
        if profile in {"wtpdf1r", "wtpdf1a"} and not profile_report.get(
            "pdf_declaration_present"
        ):
            checks.append("missing_pdf_declaration")
        if profile in {"pdfa1a", "pdfa2a", "pdfa3a"}:
            for key in ["struct_tree_root_present", "mark_info_present", "lang_present"]:
                if not profile_report.get(key):
                    checks.append(f"missing_{key}")
        if profile in {"pdfua1", "pdfua2", "wtpdf1r", "wtpdf1a"}:
            for key in ["struct_tree_root_present", "mark_info_present", "lang_present"]:
                if not profile_report.get(key):
                    checks.append(f"missing_{key}")
        if profile == "pdfvt1":
            for key in PDFVT_DPART_INSPECTION_KEYS:
                if not profile_report.get(key):
                    checks.append(f"missing_{key}")
        checks.extend(profile_report.get("seed_blockers", []))
    else:
        checks.append("inspect_command_failed")
    return {
        "exit": exit_code,
        "ok": exit_code == 0 and not checks,
        "report": str(report_path),
        "stderr": str(err_path),
        "checks": checks,
        "page_count": report.get("page_count") if report else None,
        "profile": report.get("profile") if report else None,
    }


def inspect_jit(profile: str, out_dir: Path) -> dict[str, object]:
    path = out_dir / f"{profile}.jit.jsonl"
    checks: list[str] = []
    event: dict[str, object] | None = None
    if not path.exists():
        return {"ok": False, "checks": ["missing_jit"], "event": None}
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("type") == "jit.pdf_profile":
            event = row
            break
    if event is None:
        checks.append("missing_jit_pdf_profile_event")
    else:
        if event.get("pdf_profile") != profile:
            checks.append("jit_profile_mismatch")
        if not event.get("metadata"):
            checks.append("jit_missing_metadata")
        if profile in OUTPUT_INTENT_PROFILES and not event.get("output_intent"):
            checks.append("jit_missing_output_intent")
        if profile in OUTPUT_INTENT_PROFILES and not event.get("requires_output_intent"):
            checks.append("jit_missing_output_intent_requirement")
        if profile in EMBEDDED_FONT_PROFILES and not event.get("requires_embedded_fonts"):
            checks.append("jit_missing_embedded_font_requirement")
        if profile in TAGGED_STRUCTURE_PROFILES:
            if not event.get("tagged_structure"):
                checks.append("jit_missing_tagged_structure")
            if not event.get("struct_tree_root"):
                checks.append("jit_missing_struct_tree_root")
        if profile in {"wtpdf1r", "wtpdf1a"} and not event.get("pdf_declaration"):
            checks.append("jit_missing_pdf_declaration")
        if profile == "pdfvt1" and not event.get("pdfvt_dpart_root"):
            checks.append("jit_missing_pdfvt_dpart_root")
        if profile == "pdfa4f" and not event.get("embedded_files"):
            checks.append("jit_missing_embedded_files")
    return {"ok": not checks, "checks": checks, "event": event}


def download_verapdf_classpath(cache_dir: Path) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    zip_path = cache_dir / "verapdf-installer.zip"
    if not zip_path.exists():
        urllib.request.urlretrieve(VERAPDF_URL, zip_path)
    cli_dir = cache_dir / ("verapdf-cli-" + sha256_file(zip_path)[:16].lower())
    main_class = cli_dir / "org/verapdf/apps/GreenfieldCliWrapper.class"
    if not main_class.is_file():
        with zipfile.ZipFile(zip_path) as installer_zip:
            jars = sorted(name for name in installer_zip.namelist() if name.endswith(".jar"))
            if not jars:
                raise RuntimeError("veraPDF installer jar not found in download")
            jar_bytes = installer_zip.read(jars[0])
        with zipfile.ZipFile(io.BytesIO(jar_bytes)) as jar:
            pack_names = [name for name in jar.namelist() if name.endswith("pack-veraPDF CLI")]
            if not pack_names:
                raise RuntimeError("veraPDF CLI pack not found in installer jar")
            pack_bytes = jar.read(pack_names[0])
        cli_dir.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(io.BytesIO(pack_bytes)) as pack:
            pack.extractall(cli_dir)
        if not main_class.is_file():
            raise RuntimeError("veraPDF download did not contain the CLI entrypoint")
    return cli_dir


def verapdf_command(args: argparse.Namespace) -> list[str] | None:
    if args.verapdf_cp:
        return ["java", "-cp", args.verapdf_cp, "org.verapdf.apps.GreenfieldCliWrapper"]
    if args.verapdf_cmd:
        return args.verapdf_cmd.split()
    found = shutil.which("verapdf") or shutil.which("verapdf.bat")
    if found:
        return [found]
    if args.download_verapdf:
        cli_dir = download_verapdf_classpath(Path(args.cache_dir))
        return ["java", "-cp", str(cli_dir), "org.verapdf.apps.GreenfieldCliWrapper"]
    return None


def validate_with_verapdf(
    profile: str, pdf_path: Path, out_dir: Path, command: list[str] | None
) -> dict[str, object]:
    if profile not in VERAPDF_FLAVOURS:
        return {"status": "not_applicable"}
    report_path = out_dir / f"{profile}.verapdf.json"
    err_path = out_dir / f"{profile}.verapdf.stderr.txt"
    if command is None:
        return {"status": "skipped", "reason": "verapdf_not_available"}
    exit_code = run_capture(
        command
        + [
            "--format",
            "json",
            "--maxfailuresdisplayed",
            "-1",
            "-f",
            VERAPDF_FLAVOURS[profile],
            str(pdf_path),
        ],
        report_path,
        err_path,
    )
    try:
        report = decode_json_report(report_path)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        return {
            "status": "failed",
            "exit": exit_code,
            "flavour": VERAPDF_FLAVOURS[profile],
            "compliant": False,
            "failed_rules": 0,
            "failed_checks": 0,
            "exception": f"{type(exc).__name__}: {exc}",
            "report": str(report_path),
            "stderr": str(err_path),
        }
    report_root = report.get("report", {}) if isinstance(report, dict) else {}
    jobs = report_root.get("jobs", []) if isinstance(report_root, dict) else []
    job = jobs[0] if jobs and isinstance(jobs[0], dict) else {}
    task_exception = job.get("taskException")
    validation_results = job.get("validationResult", [])
    if task_exception or not validation_results:
        return {
            "status": "failed",
            "exit": exit_code,
            "flavour": VERAPDF_FLAVOURS[profile],
            "compliant": False,
            "failed_rules": 0,
            "failed_checks": 0,
            "exception": task_exception,
            "report": str(report_path),
            "stderr": str(err_path),
        }
    result = validation_results[0]
    if not isinstance(result, dict):
        return {
            "status": "failed",
            "exit": exit_code,
            "flavour": VERAPDF_FLAVOURS[profile],
            "compliant": False,
            "failed_rules": 0,
            "failed_checks": 0,
            "exception": "veraPDF validationResult was not an object",
            "report": str(report_path),
            "stderr": str(err_path),
        }
    details = result.get("details", {})
    if not isinstance(details, dict):
        details = {}
    return {
        "status": "passed" if result.get("compliant") else "failed",
        "exit": exit_code,
        "flavour": VERAPDF_FLAVOURS[profile],
        "profile_name": result.get("profileName"),
        "compliant": bool(result.get("compliant")),
        "failed_rules": int(details.get("failedRules", 0)),
        "failed_checks": int(details.get("failedChecks", 0)),
        "report": str(report_path),
        "stderr": str(err_path),
    }


def import_pdf_oxide(args: argparse.Namespace):
    try:
        return importlib.import_module("pdf_oxide")
    except ImportError:
        pass
    target = Path(args.cache_dir) / f"pdf_oxide-{args.pdf_oxide_version}"
    if target.exists():
        sys.path.insert(0, str(target))
        try:
            return importlib.import_module("pdf_oxide")
        except ImportError:
            pass
    if args.install_pdf_oxide:
        target.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                sys.executable,
                "-m",
                "pip",
                "install",
                "--target",
                str(target),
                "--upgrade",
                f"pdf_oxide=={args.pdf_oxide_version}",
            ],
            cwd=REPO_ROOT,
            check=True,
        )
        sys.path.insert(0, str(target))
        return importlib.import_module("pdf_oxide")
    return None


def validate_with_pdf_oxide(profile: str, pdf_path: Path, out_dir: Path, module) -> dict[str, object]:
    if profile not in PDFX_PROFILES:
        return {"status": "not_applicable"}
    report_path = out_dir / f"{profile}.pdf_oxide_pdfx4.json"
    if module is None:
        return {"status": "skipped", "reason": "pdf_oxide_not_available"}
    document = module.PdfDocument(str(pdf_path))
    result = document.validate_pdf_x("4")
    write_json(report_path, result)
    errors = result.get("errors", [])
    warnings = result.get("warnings", [])
    return {
        "status": "passed" if result.get("valid") and not errors and not warnings else "failed",
        "valid": bool(result.get("valid")),
        "errors": errors,
        "warnings": warnings,
        "report": str(report_path),
    }


def validate_with_pdfvt_command(
    profile: str, pdf_path: Path, out_dir: Path, command_template: str | None
) -> dict[str, object]:
    if profile != "pdfvt1":
        return {"status": "not_applicable"}
    if not command_template:
        return {"status": "skipped", "reason": "dedicated_pdfvt_validator_not_configured"}
    if "{pdf}" not in command_template:
        return {"status": "failed", "reason": "validator_command_requires_pdf_placeholder"}
    report_path = out_dir / f"{profile}.pdfvt-preflight.report.json"
    stdout_path = out_dir / f"{profile}.pdfvt-preflight.stdout.txt"
    stderr_path = out_dir / f"{profile}.pdfvt-preflight.stderr.txt"
    formatted = command_template.replace("{pdf}", str(pdf_path)).replace("{report}", str(report_path))
    try:
        # Split the configured command before substituting paths. Spaces and
        # backslashes in a PDF path must never become extra process arguments.
        argv = [arg.replace("{pdf}", str(pdf_path)).replace("{report}", str(report_path))
                for arg in shlex.split(command_template)]
        exit_code = run_capture(argv, stdout_path, stderr_path)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        return {"status": "failed", "reason": "validator_launch_failed", "error": str(error)}
    return {
        "status": "passed" if exit_code == 0 else "failed",
        "exit": exit_code,
        "command": formatted,
        "argv": argv,
        "pdf_sha256": sha256_file(pdf_path),
        "report": str(report_path) if report_path.exists() else None,
        "stdout": str(stdout_path),
        "stderr": str(stderr_path),
    }


def pdfvt_validator_version(command: str | None, out_dir: Path) -> dict[str, object]:
    if not command:
        return {"status": "skipped", "reason": "validator_version_command_not_configured"}
    stdout = out_dir / "pdfvt-validator-version.stdout.txt"
    stderr = out_dir / "pdfvt-validator-version.stderr.txt"
    try:
        code = run_capture(shlex.split(command), stdout, stderr)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        return {"status": "failed", "error": str(error)}
    return {"status": "passed" if code == 0 and (stdout.stat().st_size or stderr.stat().st_size) else "failed",
            "exit": code, "command": command, "stdout": str(stdout), "stderr": str(stderr)}


def replay_determinism(
    profile: str,
    baseline_hash: str,
    replay_dir: Path,
    font_path: Path,
    icc_path: Path,
    *,
    html: str = HTML_SPECIMEN,
    css: str = CSS_SPECIMEN,
    title: str | None = None,
) -> dict[str, object]:
    replay_dir.mkdir(parents=True, exist_ok=True)
    replay = render_profile(
        profile,
        replay_dir,
        font_path,
        icc_path,
        emit_observability=False,
        html=html,
        css=css,
        title=title,
    )
    replay_hash = replay.get("sha256")
    return {
        "exit": replay["exit"],
        "deterministic": replay["exit"] == 0 and replay_hash == baseline_hash,
        "baseline_sha256": baseline_hash,
        "replay_sha256": replay_hash,
        "pdf": replay["pdf"],
    }


def validate_pdfvt_multipage(
    profile: str,
    out_dir: Path,
    font_path: Path,
    icc_path: Path,
    pdf_oxide_module,
    pdfvt_command: str | None = None,
) -> dict[str, object]:
    if profile != "pdfvt1":
        return {"status": "not_applicable"}

    specimen_dir = out_dir / "supplemental" / "pdfvt_multipage"
    specimen_dir.mkdir(parents=True, exist_ok=True)
    title = "FullBleed pdfvt1 multipage DPart specimen"
    render = render_profile(
        profile,
        specimen_dir,
        font_path,
        icc_path,
        emit_observability=True,
        html=PDFVT_MULTIPAGE_HTML_SPECIMEN,
        css=PDFVT_MULTIPAGE_CSS_SPECIMEN,
        title=title,
    )
    pdf_path = Path(str(render["pdf"]))
    if render["exit"] != 0:
        return {"status": "failed", "checks": ["render_failed"], "render": render}

    inspect = inspect_profile(profile, pdf_path, specimen_dir)
    jit = inspect_jit(profile, specimen_dir)
    pdf_oxide = validate_with_pdf_oxide(profile, pdf_path, specimen_dir, pdf_oxide_module)
    dedicated = validate_with_pdfvt_command(profile, pdf_path, specimen_dir, pdfvt_command)
    determinism = replay_determinism(
        profile,
        str(render["sha256"]),
        specimen_dir / "replay",
        font_path,
        icc_path,
        html=PDFVT_MULTIPAGE_HTML_SPECIMEN,
        css=PDFVT_MULTIPAGE_CSS_SPECIMEN,
        title=title,
    )

    checks: list[str] = []
    if not inspect["ok"]:
        checks.extend(f"inspect:{check}" for check in inspect["checks"])
    if not jit["ok"]:
        checks.extend(f"jit:{check}" for check in jit["checks"])
    if int(inspect.get("page_count") or 0) < 2:
        checks.append("expected_at_least_two_pages")
    profile_report = inspect.get("profile") or {}
    if isinstance(profile_report, dict):
        for key in PDFVT_DPART_INSPECTION_KEYS:
            if not profile_report.get(key):
                checks.append(f"missing_{key}")
    else:
        checks.append("missing_profile_report")
    if pdf_oxide.get("status") == "failed":
        checks.append("pdf_oxide_pdfx4_failed")
    if dedicated.get("status") == "failed":
        checks.append("dedicated_pdfvt_failed")
    if not determinism.get("deterministic"):
        checks.append("determinism_failed")

    return {
        "status": "passed" if not checks else "failed",
        "checks": checks,
        "render": render,
        "inspect": inspect,
        "jit": jit,
        "pdf_oxide_pdfx4": pdf_oxide,
        "dedicated_pdfvt": dedicated,
        "determinism": determinism,
    }


def validate_pdfvt_composition(out_dir, font_path, icc_path, pdf_oxide, command):
    """Exercise grouped records, DPM, reuse, fixed bindings and content reflow."""
    import base64
    import struct
    import zlib
    import fullbleed

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"\x00\x20\x50\x90")) + chunk(b"IEND", b"")
    logo = '<img width="24" height="24" src="data:image/png;base64,' + base64.b64encode(png).decode() + '">'
    css = "@page{size:300pt 300pt;margin:20pt} body{font-family:Inter;font-size:10pt}"
    job = {"id": "release-validation", "metadata": {"product": "statements", "copies": 1}, "records": [
        {"id": "000001", "metadata": {"postal_code": "60601", "duplex": True},
         "documents": [{"id": "statement"}, {"id": "insert", "metadata": {"media": "insert"}}]},
        {"id": "000002", "documents": [{"id": "statement"}]},
    ]}
    def engine(metadata=None):
        return fullbleed.PdfEngine(pdf_profile="pdfvt1", document_title="Fullbleed PDF/VT composition specimen",
            document_timestamp="2026-09-30T00:00:00Z", output_intent_icc=str(icc_path),
            output_intent_identifier="sRGB IEC61966-2.1", font_files=[str(font_path)], pdf_vt_job=metadata)
    def grouped():
        return engine(job).render_pdf_batch([logo + "<p>Statement A</p>", logo + "<p>Insert A</p>", logo + "<p>Statement B</p>"], css)
    def fixed():
        return engine().compile_pdf(logo + "<p>{{name}}</p>", css).render_pdf_bindings({"name": ["Alice", "Bob"]})
    def reflow():
        return engine().compile_pdf(logo + "<p>{{name}}</p>", css).render_pdf_reflow_bindings({"name": ["Alice", "Bob " * 1000]})
    results = {}
    for name, render in [("grouped", grouped), ("fixed", fixed), ("reflow", reflow)]:
        folder = out_dir / "supplemental" / f"pdfvt_{name}"
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / "pdfvt1.pdf"
        try:
            pdf = render()
            path.write_bytes(pdf)
            inspection = fullbleed.inspect_pdf(str(path))
            write_json(folder / "inspection.json", inspection)
            profile = inspection["profile"]
            replay = render()
            (folder / "replay.pdf").write_bytes(replay)
            dedicated = validate_with_pdfvt_command("pdfvt1", path, folder, command)
            substrate = validate_with_pdf_oxide("pdfvt1", path, folder, pdf_oxide)
            ok = (not profile["seed_blockers"] and profile["pdfvt_record_count"] == 2
                  and profile["pdfvt_document_count"] == (3 if name == "grouped" else 2)
                  and profile["pdfvt_reuse_hint_count"] > 0 and profile["pdfvt_dpm_valid"]
                  and pdf == replay and dedicated["status"] != "failed" and substrate["status"] != "failed")
            results[name] = {"status": "passed" if ok else "failed", "pdf": str(path),
                "sha256": sha256_file(path), "deterministic": pdf == replay,
                "inspection": inspection, "dedicated_pdfvt": dedicated, "pdf_oxide_pdfx4": substrate}
        except Exception as error:
            results[name] = {"status": "failed", "error": str(error)}
    return {"status": "passed" if all(r["status"] == "passed" for r in results.values()) else "failed", "specimens": results}


def validate_pdfvt_negative_controls(pdf_path, out_dir, command):
    if not command:
        return {"status": "skipped", "reason": "dedicated_pdfvt_validator_not_configured"}
    data = pdf_path.read_bytes()
    results = {}
    for name, before, after in [
        ("missing_dpart_root_node", b"/DPartRootNode ", b"/DPartRootGone "),
        ("wrong_pdf_version", b"%PDF-1.6", b"%PDF-1.7"),
        ("mismatched_vt_date", b'pdfvtid:GTS_PDFVTModDate="2026-09-30', b'pdfvtid:GTS_PDFVTModDate="2026-09-29'),
    ]:
        folder = out_dir / "negative-controls" / name
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / "invalid.pdf"
        if before not in data or len(before) != len(after):
            results[name] = {"status": "failed", "reason": "mutation_not_applied"}
            continue
        path.write_bytes(data.replace(before, after, 1))
        validation = validate_with_pdfvt_command("pdfvt1", path, folder, command)
        # A crashed/missing validator is not evidence that a bad PDF was rejected.
        rejected = validation.get("exit") == 1
        results[name] = {"status": "passed" if rejected else "failed", "validation": validation}
    return {"status": "passed" if all(r["status"] == "passed" for r in results.values()) else "failed", "controls": results}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default=str(REPO_ROOT / "output" / "conformance_validation"))
    parser.add_argument("--profile", action="append", choices=DEFAULT_PROFILES)
    parser.add_argument("--font", default=str(default_font_path()))
    parser.add_argument("--icc")
    parser.add_argument(
        "--strict-external",
        action="store_true",
        help="Fail if veraPDF or PDF/X validation is unavailable.",
    )
    parser.add_argument("--verapdf-cp", help="Classpath directory containing veraPDF CLI classes.")
    parser.add_argument("--verapdf-cmd", help="Explicit veraPDF command.")
    parser.add_argument("--download-verapdf", action="store_true")
    parser.add_argument("--install-pdf-oxide", action="store_true")
    parser.add_argument("--pdf-oxide-version", default="0.3.78")
    parser.add_argument(
        "--pdfvt-version-cmd", default=os.environ.get("FULLBLEED_PDFVT_VALIDATOR_VERSION_CMD"),
        help="Command that identifies the dedicated validator version and validation profile; its output is retained.",
    )
    parser.add_argument(
        "--pdfvt-cmd",
        default=os.environ.get("FULLBLEED_PDFVT_VALIDATOR_CMD"),
        help=(
            "Dedicated PDF/VT preflight command. Use {pdf} for input path and "
            "{report} for an optional native JSON report path. Stdout/stderr are retained separately. Exit code 0 means pass."
        ),
    )
    parser.add_argument(
        "--require-dedicated-pdfvt",
        action="store_true",
        help="Require dedicated validation of every VT specimen, rejected negative controls, and validator version evidence.",
    )
    parser.add_argument(
        "--cache-dir",
        default=str(Path(tempfile.gettempdir()) / "fullbleed-pdf-profile-validators"),
    )
    args = parser.parse_args()

    profiles = args.profile or DEFAULT_PROFILES
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    font_path = Path(args.font)
    if not font_path.exists():
        raise SystemExit(f"Font not found: {font_path}")
    icc_path = resolve_icc(args.icc)

    verapdf = verapdf_command(args)
    pdf_oxide = import_pdf_oxide(args)
    validator_version = pdfvt_validator_version(args.pdfvt_version_cmd, out_dir) if "pdfvt1" in profiles else {"status": "not_applicable"}
    summary: dict[str, object] = {
        "schema": "fullbleed.pdf_profile_conformance.v1",
        "ok": True,
        "runtime": {
            "fullbleed_version": importlib.metadata.version("fullbleed"),
            "python": sys.version,
            "platform": platform.platform(),
        },
        "profiles": {},
        "validators": {
            "verapdf": {"available": verapdf is not None, "command": verapdf},
            "pdf_oxide": {"available": pdf_oxide is not None, "version": getattr(pdf_oxide, "__version__", None)},
            "dedicated_pdfvt": {
                "available": args.pdfvt_cmd is not None,
                "command": args.pdfvt_cmd,
                "version_evidence": validator_version,
                "note": "Internal writer checks and PDF/X-4 validation do not replace dedicated PDF/VT validation.",
            },
        },
        "inputs": {
            "font": str(font_path),
            "icc": str(icc_path),
            "icc_sha256": sha256_file(icc_path),
            "font_sha256": sha256_file(font_path),
            "out": str(out_dir),
        },
    }

    replay_dir = out_dir / "replay"
    for profile in profiles:
        render = render_profile(
            profile, out_dir, font_path, icc_path, emit_observability=True
        )
        pdf_path = Path(str(render["pdf"]))
        inspect = inspect_profile(profile, pdf_path, out_dir) if render["exit"] == 0 else None
        jit = inspect_jit(profile, out_dir) if render["exit"] == 0 else None
        verapdf_result = (
            validate_with_verapdf(profile, pdf_path, out_dir, verapdf)
            if render["exit"] == 0
            else {"status": "skipped", "reason": "render_failed"}
        )
        pdf_oxide_result = (
            validate_with_pdf_oxide(profile, pdf_path, out_dir, pdf_oxide)
            if render["exit"] == 0
            else {"status": "skipped", "reason": "render_failed"}
        )
        pdfvt_result = (
            validate_with_pdfvt_command(profile, pdf_path, out_dir, args.pdfvt_cmd)
            if render["exit"] == 0
            else {"status": "skipped", "reason": "render_failed"}
        )
        pdfvt_multipage = (
            validate_pdfvt_multipage(profile, out_dir, font_path, icc_path, pdf_oxide, args.pdfvt_cmd)
            if render["exit"] == 0
            else {"status": "skipped", "reason": "render_failed"}
        )
        determinism = (
            replay_determinism(
                profile, str(render["sha256"]), replay_dir, font_path, icc_path
            )
            if render["exit"] == 0 and render["sha256"]
            else {"deterministic": False, "reason": "render_failed"}
        )
        profile_ok = (
            render["exit"] == 0
            and bool(inspect and inspect["ok"])
            and bool(jit and jit["ok"])
            and bool(determinism.get("deterministic"))
        )
        composition = ({"status": "not_applicable"} if profile != "pdfvt1" else
                       validate_pdfvt_composition(out_dir, font_path, icc_path, pdf_oxide, args.pdfvt_cmd))
        negative_controls = ({"status": "not_applicable"} if profile != "pdfvt1" or render["exit"] != 0 else
                             validate_pdfvt_negative_controls(pdf_path, out_dir, args.pdfvt_cmd))
        if composition["status"] == "failed" or negative_controls["status"] == "failed":
            profile_ok = False
        for external in [verapdf_result, pdf_oxide_result]:
            status = external.get("status")
            if status == "failed":
                profile_ok = False
            if args.strict_external and status == "skipped":
                profile_ok = False
        if pdfvt_result.get("status") == "failed":
            profile_ok = False
        if pdfvt_multipage.get("status") == "failed":
            profile_ok = False
        if (
            args.require_dedicated_pdfvt
            and profile == "pdfvt1"
            and (pdfvt_result.get("status") != "passed"
                 or validator_version["status"] != "passed"
                 or pdfvt_multipage.get("dedicated_pdfvt", {}).get("status") != "passed"
                 or negative_controls["status"] != "passed"
                 or any(s.get("dedicated_pdfvt", {}).get("status") != "passed"
                        for s in composition.get("specimens", {}).values()))
        ):
            profile_ok = False
        summary["profiles"][profile] = {
            "ok": profile_ok,
            "render": render,
            "inspect": inspect,
            "jit": jit,
            "verapdf": verapdf_result,
            "pdf_oxide_pdfx4": pdf_oxide_result,
            "dedicated_pdfvt": pdfvt_result,
            "pdfvt_multipage": pdfvt_multipage,
            "pdfvt_composition": composition,
            "pdfvt_negative_controls": negative_controls,
            "determinism": determinism,
        }
        if not profile_ok:
            summary["ok"] = False

    write_json(out_dir / "validation-summary.json", summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
