# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Fullbleed public CLI entrypoint.

COMPLIANCE_SPEC: fullbleed.cli_compliance.v1
PACKAGE_LICENSE: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
COPYRIGHT_FILE: COPYRIGHT
THIRD_PARTY_NOTICE_FILE: THIRD_PARTY_LICENSES.md
THIRD_PARTY_ALLOWED_LICENSES: OFL-1.1, Apache-2.0, MIT, UFL-1.0
LICENSE_AUDIT_ARTIFACTS: FONT_LICENSE_AUDIT.md, FONT_LICENSE_AUDIT.json
AUTO_COMPLIANCE_FLAG_CODES:
  - LIC_MISSING_NOTICE
  - LIC_POLICY_MISMATCH
  - LIC_DISALLOWED
  - LIC_UNKNOWN
  - LIC_AUDIT_STALE
  - LIC_ASSET_UNMAPPED
  - LIC_COMMERCIAL_UNATTESTED
"""
import argparse
import importlib.metadata as metadata
import hashlib
import json
import os
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
import re

import fullbleed
import fullbleed_assets


PAGE_SIZES = {
    "letter": ("8.5in", "11in"),
    "a4": ("210mm", "297mm"),
    "legal": ("8.5in", "14in"),
}

# Profile presets: dev (fast/verbose), preflight (validation), prod (optimized)
PROFILES = {
    "dev": {
        "reuse_xobjects": False,
        "jit_mode": "plan",
    },
    "preflight": {
        "jit_mode": "plan",
        "reuse_xobjects": True,
    },
    "prod": {
        "reuse_xobjects": True,
        "jit_mode": "off",
    },
}

FAIL_ON_CHOICES = ["overflow", "missing-glyphs", "font-subst", "budget"]
WATERMARK_LAYER_ALIASES = {"underlay": "background"}
WATERMARK_LAYER_CHOICES = {"background", "overlay"}
LICENSE_SPDX_EXPRESSION = "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial"
LICENSE_OPEN_SOURCE = "AGPL-3.0-only"
LICENSE_COMMERCIAL_REF = "LicenseRef-Fullbleed-Commercial"
COMPLIANCE_POLICY = {
    "schema": "fullbleed.cli_compliance.v1",
    "package_license": LICENSE_SPDX_EXPRESSION,
    "license_options": [LICENSE_OPEN_SOURCE, LICENSE_COMMERCIAL_REF],
    "license_file": "LICENSE",
    "license_required_header": "GNU AFFERO GENERAL PUBLIC LICENSE",
    "license_forbidden_markers": ["Apache License"],
    "licensing_guide_file": "LICENSING.md",
    "commercial_contact_email": "info@fullbleed.dev",
    "commercial_contact_web": "fullbleed.dev",
    "copyright_file": "COPYRIGHT",
    "copyright_required_spdx": LICENSE_SPDX_EXPRESSION,
    "copyright_required_markers": [
        "dual-licensed",
        "AGPL-3.0-only",
        "LicenseRef-Fullbleed-Commercial",
        "LICENSE",
        "LICENSING.md",
    ],
    "third_party_notice_file": "THIRD_PARTY_LICENSES.md",
    "third_party_allowed_licenses": ["OFL-1.1", "Apache-2.0", "MIT", "UFL-1.0"],
    "license_audit_artifacts": ["FONT_LICENSE_AUDIT.md", "FONT_LICENSE_AUDIT.json"],
    "flag_codes": [
        "LIC_MISSING_NOTICE",
        "LIC_POLICY_MISMATCH",
        "LIC_DISALLOWED",
        "LIC_UNKNOWN",
        "LIC_AUDIT_STALE",
        "LIC_ASSET_UNMAPPED",
        "LIC_COMMERCIAL_UNATTESTED",
    ],
}

SCHEMA_REGISTRY = {
    "render": "fullbleed.render_result.v1",
    "verify": "fullbleed.verify_result.v1",
    "plan": "fullbleed.plan_result.v1",
    "run": "fullbleed.run_result.v1",
    "compliance": "fullbleed.compliance.v1",
    "capabilities": "fullbleed.capabilities.v1",
    "debug-perf": "fullbleed.debug_perf.v1",
    "debug-jit": "fullbleed.debug_jit.v1",
    "doctor": "fullbleed.doctor.v1",
    "assets:list": "fullbleed.assets_list.v1",
    "assets:info": "fullbleed.assets_info.v1",
    "assets:install": "fullbleed.assets_install.v1",
    "assets:verify": "fullbleed.assets_verify.v1",
    "assets:lock": "fullbleed.assets_lock.v1",
    "cache:dir": "fullbleed.cache_dir.v1",
    "cache:prune": "fullbleed.cache_prune.v1",
    "init": "fullbleed.init.v1",
    "new": "fullbleed.new_template.v1",
}

SCHEMA_DEFS = {
    "fullbleed.render_result.v1": {
        "type": "object",
        "required": ["schema", "ok", "outputs"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "bytes_written": {"type": "integer"},
            "outputs": {"type": "object"},
            "code": {"type": "string"},
            "message": {"type": "string"},
            "failures": {"type": "array"},
        },
    },
    "fullbleed.verify_result.v1": {
        "type": "object",
        "required": ["schema", "ok", "outputs"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "bytes_written": {"type": "integer"},
            "outputs": {"type": "object"},
        },
    },
    "fullbleed.plan_result.v1": {
        "type": "object",
        "required": ["schema", "ok", "manifest"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "manifest": {"type": "object"},
            "warnings": {"type": "array"},
        },
    },
    "fullbleed.run_result.v1": {
        "type": "object",
        "required": ["schema", "ok", "entrypoint", "outputs"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "entrypoint": {"type": "string"},
            "bytes_written": {"type": "integer"},
            "license_warning_emitted": {"type": "boolean"},
            "outputs": {"type": "object"},
        },
    },
    "fullbleed.compliance.v1": {
        "type": "object",
        "required": ["schema", "ok", "policy", "license", "flags"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "policy": {"type": "object"},
            "license": {"type": "object"},
            "runtime": {"type": "object"},
            "files": {"type": "object"},
            "audit": {"type": "object"},
            "flags": {"type": "array"},
        },
    },
    "fullbleed.error.v1": {
        "type": "object",
        "required": ["schema", "ok", "code", "message"],
        "properties": {
            "schema": {"type": "string"},
            "ok": {"type": "boolean"},
            "code": {"type": "string"},
            "message": {"type": "string"},
        },
    },
    "fullbleed.assets_list.v1": {
        "type": "object",
        "required": ["schema", "packages"],
        "properties": {"schema": {"type": "string"}, "packages": {"type": "array"}},
    },
    "fullbleed.assets_info.v1": {
        "type": "object",
        "required": ["schema", "name", "version"],
        "properties": {"schema": {"type": "string"}, "name": {"type": "string"}, "version": {"type": "string"}},
    },
    "fullbleed.assets_install.v1": {
        "type": "object",
        "required": ["schema", "ok", "name", "version", "installed_to"],
        "properties": {"schema": {"type": "string"}, "ok": {"type": "boolean"}},
    },
    "fullbleed.assets_verify.v1": {
        "type": "object",
        "required": ["schema", "ok", "name"],
        "properties": {"schema": {"type": "string"}, "ok": {"type": "boolean"}, "name": {"type": "string"}},
    },
    "fullbleed.assets_lock.v1": {
        "type": "object",
        "required": ["schema", "ok", "path", "packages"],
        "properties": {"schema": {"type": "string"}, "ok": {"type": "boolean"}, "path": {"type": "string"}},
    },
    "fullbleed.cache_dir.v1": {
        "type": "object",
        "required": ["schema", "path", "exists"],
        "properties": {"schema": {"type": "string"}, "path": {"type": "string"}, "exists": {"type": "boolean"}},
    },
    "fullbleed.cache_prune.v1": {
        "type": "object",
        "required": ["schema", "dry_run", "removed_count"],
        "properties": {"schema": {"type": "string"}, "removed": {"type": "array"}},
    },
    "fullbleed.init.v1": {
        "type": "object",
        "required": ["schema", "ok", "path"],
        "properties": {"schema": {"type": "string"}, "ok": {"type": "boolean"}, "path": {"type": "string"}},
    },
    "fullbleed.new_template.v1": {
        "type": "object",
        "required": ["schema", "ok", "template"],
        "properties": {"schema": {"type": "string"}, "ok": {"type": "boolean"}, "template": {"type": "string"}},
    },
    "fullbleed.doctor.v1": {
        "type": "object",
        "required": ["python", "platform", "pdf_versions"],
        "properties": {"python": {"type": "string"}, "platform": {"type": "string"}},
    },
    "fullbleed.capabilities.v1": {
        "type": "object",
        "required": ["schema", "commands", "agent_flags", "engine"],
        "properties": {
            "schema": {"type": "string"},
            "commands": {"type": "array"},
            "agent_flags": {"type": "array"},
            "engine": {"type": "object"},
            "svg": {"type": "object"},
        },
    },
    "fullbleed.debug_perf.v1": {
        "type": "array",
        "items": {"type": "object"},
    },
    "fullbleed.debug_jit.v1": {
        "type": "array",
        "items": {"type": "object"},
    },
    "fullbleed.repro_record.v1": {
        "type": "object",
        "required": ["schema", "input_fingerprint_sha256", "output_pdf_sha256"],
        "properties": {
            "schema": {"type": "string"},
            "cli_version": {"type": "string"},
            "input_manifest_sha256": {"type": "string"},
            "input_fingerprint_sha256": {"type": "string"},
            "output_pdf_sha256": {"type": "string"},
            "output_bytes_written": {"type": "integer"},
        },
    },
}

def _get_version():
    """Return installed Fullbleed version, with a dev fallback."""
    for dist in ("fullbleed", "fullbleed-cli"):
        try:
            return metadata.version(dist)
        except metadata.PackageNotFoundError:
            continue
    return "0.0.0-dev"


def _compliance_roots():
    """Candidate directories where compliance docs might exist."""
    roots = []
    for candidate in [Path.cwd(), *Path(__file__).resolve().parents[:4]]:
        try:
            resolved = candidate.resolve()
        except Exception:
            continue
        if resolved not in roots:
            roots.append(resolved)
    return roots


def _find_compliance_file(rel_name):
    """Locate a compliance file by searching known repository roots."""
    for root in _compliance_roots():
        path = root / rel_name
        if path.exists():
            return path
    return None


def _is_truthy(value):
    """Return True for common truthy string/integer environment values."""
    return str(value or "").strip().lower() in {"1", "true", "yes", "on"}


def _load_commercial_license_file(path):
    """Parse a commercial attestation file (JSON object/string or plain text id)."""
    p = Path(path).expanduser()
    text = p.read_text(encoding="utf-8").strip()
    payload = {}
    if not text:
        return payload
    try:
        obj = json.loads(text)
    except Exception:
        payload["license_id"] = text
        return payload
    if isinstance(obj, dict):
        for key in ("license_id", "tier", "company"):
            if key in obj and obj.get(key) is not None:
                payload[key] = str(obj.get(key))
    elif isinstance(obj, str):
        payload["license_id"] = obj
    return payload


def _resolve_license_attestation(args=None):
    """Resolve effective commercial license attestation from args/env/file."""
    mode = str(
        getattr(args, "license_mode", None)
        or os.environ.get("FULLBLEED_LICENSE_MODE")
        or "auto"
    ).strip().lower()
    if mode not in {"auto", "agpl", "commercial"}:
        mode = "auto"

    arg_license_id = getattr(args, "commercial_license_id", None)
    env_license_id = (
        os.environ.get("FULLBLEED_COMMERCIAL_LICENSE_ID")
        or os.environ.get("FULLBLEED_COMMERCIAL_LICENSE")
    )
    arg_license_file = getattr(args, "commercial_license_file", None)
    env_license_file = os.environ.get("FULLBLEED_COMMERCIAL_LICENSE_FILE")
    arg_company = getattr(args, "commercial_company", None)
    env_company = os.environ.get("FULLBLEED_COMMERCIAL_COMPANY")
    arg_tier = getattr(args, "commercial_tier", None)
    env_tier = os.environ.get("FULLBLEED_COMMERCIAL_TIER")
    explicit_attest = bool(getattr(args, "commercial_licensed", False)) or _is_truthy(
        os.environ.get("FULLBLEED_COMMERCIAL_LICENSED")
    )

    payload = {}
    source = None
    parse_error = None
    license_file = arg_license_file or env_license_file
    if license_file:
        try:
            payload = _load_commercial_license_file(license_file)
            source = "file"
        except Exception as exc:
            parse_error = str(exc)

    license_id = arg_license_id or env_license_id or payload.get("license_id")
    company = arg_company or env_company or payload.get("company")
    tier = arg_tier or env_tier or payload.get("tier")

    if mode == "auto":
        if explicit_attest or license_id or license_file:
            mode = "commercial"
        else:
            mode = "agpl"

    attested = False
    if mode == "commercial":
        attested = bool(explicit_attest or license_id or source == "file")

    return {
        "mode": mode,
        "attested": attested,
        "license_id": license_id,
        "company": company,
        "tier": tier,
        "source": source or ("arg_or_env" if license_id else None),
        "file": str(Path(license_file).expanduser()) if license_file else None,
        "parse_error": parse_error,
    }


def _license_warning_state_path():
    """Return persisted marker path for one-time `run` license warning."""
    base = os.environ.get("FULLBLEED_STATE_DIR")
    if base:
        state_dir = Path(base).expanduser()
    else:
        state_dir = Path.home() / ".fullbleed"
    return state_dir / "notices" / "run_license_warn_v1.ack"


def _maybe_emit_run_license_warning(args):
    """Emit a one-time AGPL/commercial reminder for `fullbleed run`."""
    if getattr(args, "no_license_warn", False):
        return False
    if os.environ.get("FULLBLEED_NO_LICENSE_WARN", "").strip().lower() in {"1", "true", "yes"}:
        return False
    license_state = _resolve_license_attestation(None)
    if license_state["mode"] == "commercial" and license_state["attested"]:
        return False
    marker = _license_warning_state_path()
    if marker.exists():
        return False

    sys.stderr.write(
        "[license] fullbleed is dual-licensed (AGPL-3.0-only OR commercial). "
        "Using `fullbleed run` loads and executes your Python code against fullbleed APIs. "
        "For commercial licensing: info@fullbleed.dev\n"
    )
    try:
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text(
            json.dumps(
                {
                    "schema": "fullbleed.license_notice_ack.v1",
                    "acknowledged_at_utc": datetime.now(timezone.utc).isoformat(),
                },
                ensure_ascii=True,
            )
            + "\n",
            encoding="utf-8",
        )
    except Exception:
        # Notice emission should never block rendering.
        pass
    return True


def _apply_global_flags(args):
    """Apply global CLI flags to process environment and parsed args."""
    if getattr(args, "json_only", False):
        args.json = True
        args.no_prompts = True
        os.environ["FULLBLEED_JSON_ONLY"] = "1"
    if getattr(args, "no_color", False):
        os.environ["NO_COLOR"] = "1"
    if getattr(args, "no_prompts", False):
        os.environ["FULLBLEED_NO_PROMPTS"] = "1"


def _infer_schema_from_argv(argv):
    """Infer requested schema id from argv when `--schema` is present."""
    tokens = [t for t in (argv or []) if t != "--schema"]
    command = None
    sub = None
    for t in tokens:
        if t.startswith("-"):
            continue
        command = t
        break
    if not command:
        return None
    if command in {"assets", "cache"}:
        found_command = False
        for t in tokens:
            if t == command:
                found_command = True
                continue
            if not found_command:
                continue
            if t.startswith("-"):
                continue
            sub = t
            break
        if not sub:
            return None
        return SCHEMA_REGISTRY.get(f"{command}:{sub}")
    return SCHEMA_REGISTRY.get(command)


def _emit_schema(schema_name):
    """Emit JSON schema envelope for a known schema id."""
    payload = {
        "schema": "fullbleed.schema.v1",
        "target": schema_name,
    }
    if schema_name in SCHEMA_DEFS:
        payload["definition"] = SCHEMA_DEFS[schema_name]
    sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")


def _resolve_assets(args):
    """Resolve CLI asset flags into normalized (path, kind, name) tuples."""
    assets = args.asset or []
    kinds = args.asset_kind or []
    names = args.asset_name or []

    def _resolve_asset_path(raw):
        if raw.startswith("@"):
            key = raw[1:].lower()
            if key in {"bootstrap", "bootstrap5", "bootstrap5.0.0"}:
                return (
                    str(fullbleed_assets.asset_path("bootstrap.min.css")),
                    "css",
                    "bootstrap-5.0.0",
                )
            if key in {"bootstrap-icons", "bootstrapicons", "bootstrap-icons1.11.3"}:
                return (
                    str(fullbleed_assets.asset_path("icons/bootstrap-icons.svg")),
                    "svg",
                    "bootstrap-icons-1.11.3",
                )
            if key in {"noto-sans", "noto-sans-regular"}:
                return (
                    str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")),
                    "font",
                    "NotoSans-Regular",
                )
            raise ValueError(f"unknown builtin asset: {raw}")
        return raw, None, None

    inferred = []
    for idx, asset in enumerate(assets):
        path, builtin_kind, builtin_name = _resolve_asset_path(asset)
        kind = None
        name = None
        if idx < len(kinds):
            kind = kinds[idx]
        if idx < len(names):
            name = names[idx]
        if builtin_kind and not kind:
            kind = builtin_kind
        if builtin_name and not name:
            name = builtin_name
        inferred.append((path, kind, name))

    resolved = []
    for path, kind, name in inferred:
        if kind is None:
            ext = Path(path).suffix.lower()
            if ext in {".css"}:
                kind = "css"
            elif ext in {".ttf", ".otf"}:
                kind = "font"
            elif ext in {".svg"}:
                kind = "svg"
            elif ext in {".png", ".jpg", ".jpeg"}:
                kind = "image"
            else:
                raise ValueError(f"asset kind required for {path}")
        resolved.append((path, kind, name))

    return resolved


def _apply_profile(args):
    """Apply profile defaults, only for args that weren't explicitly set."""
    profile_name = getattr(args, "profile", None)
    if not profile_name:
        return
    profile = PROFILES.get(profile_name.lower())
    if not profile:
        raise ValueError(f"unknown profile: {profile_name} (expected dev, preflight, prod)")
    for key, value in profile.items():
        attr = key.replace("-", "_")
        # Only apply if user didn't explicitly set it (check against parser defaults)
        if not getattr(args, f"_explicit_{attr}", False):
            setattr(args, attr, value)


def _read_text(path_or_dash):
    """Read UTF-8 text from path, or stdin when value is '-'."""
    if path_or_dash == "-":
        return sys.stdin.read()
    return Path(path_or_dash).read_text(encoding="utf-8")


def _read_json_or_path(value):
    """Parse JSON from inline string or from file path if it exists."""
    if value is None:
        return None
    path = Path(value)
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    return json.loads(value)


def _detect_remote_refs(html, css):
    """Return unique http(s) references detected in HTML/CSS payloads."""
    urls = []
    link_re = re.compile(r"<(link|script)[^>]+(?:href|src)=[\"'](https?://[^\"']+)[\"']",
                         re.IGNORECASE)
    urls.extend([m[1] for m in link_re.findall(html)])
    import_re = re.compile(r"@import\s+(?:url\()?['\"]?(https?://[^\"')]+)",
                           re.IGNORECASE)
    urls.extend(import_re.findall(css))
    url_re = re.compile(r"url\(['\"]?(https?://[^\"')]+)", re.IGNORECASE)
    urls.extend(url_re.findall(css))
    return sorted(set(urls))


def _resolve_page_size(args):
    """Resolve page size pair from explicit width/height or named size."""
    if args.page_width or args.page_height:
        if not args.page_width or not args.page_height:
            raise ValueError("--page-width and --page-height must be provided together")
        return args.page_width, args.page_height
    if args.page_size:
        size = PAGE_SIZES.get(args.page_size.lower())
        if not size:
            raise ValueError(f"unknown page size: {args.page_size}")
        return size
    return None, None


def _normalize_watermark_layer(raw_layer):
    """Normalize watermark layer aliases and validate allowed choices."""
    layer = (raw_layer or "overlay").strip().lower()
    layer = WATERMARK_LAYER_ALIASES.get(layer, layer)
    if layer not in WATERMARK_LAYER_CHOICES:
        choices = "', '".join(sorted(WATERMARK_LAYER_CHOICES))
        raise ValueError(
            f"Invalid watermark layer: {raw_layer!r}. Expected '{choices}' "
            "(or legacy alias 'underlay')."
        )
    return layer


def _validate_pdf_options(args):
    """Validate cross-option PDF constraints before engine construction."""
    has_output_intent_metadata = any(
        [
            getattr(args, "output_intent_identifier", None),
            getattr(args, "output_intent_info", None),
            getattr(args, "output_intent_components", None) is not None,
        ]
    )
    if has_output_intent_metadata and not getattr(args, "output_intent_icc", None):
        raise ValueError(
            "output intent metadata requires --output-intent-icc "
            "(path or data URI)."
        )


def _collect_css(args):
    """Collect CSS sources (`--css`, `--css-str`) into one stylesheet string."""
    css_parts = []
    for css_path in args.css or []:
        css_parts.append(_read_text(css_path))
    for css_str in args.css_str or []:
        css_parts.append(css_str)
    return "\n\n".join(css_parts)


def _collect_html(args):
    """Return HTML from `--html-str` or `--html` path/stdin."""
    if args.html_str is not None:
        return args.html_str
    if args.html is None:
        raise ValueError("--html or --html-str is required")
    return _read_text(args.html)


def _build_bundle(args):
    """Build and populate an `AssetBundle` from resolved CLI asset flags."""
    bundle = fullbleed.AssetBundle()
    inferred = _resolve_assets(args)

    for path, kind, name in inferred:
        asset_obj = bundle.add_file(
            path,
            kind,
            name=name,
            trusted=args.asset_trusted,
            remote=args.allow_remote_assets,
        )
        if args.json and args.verbose_assets:
            sys.stderr.write(json.dumps(asset_obj.info(), ensure_ascii=True) + "\n")

    return bundle


def _build_manifest(args):
    """Build canonical compiler-input manifest for reproducibility/debugging."""
    _apply_profile(args)
    page_width, page_height = _resolve_page_size(args)
    command_name = getattr(args, "command", None)
    if not command_name:
        command_name = "verify" if getattr(args, "emit_pdf", None) is not None else "render"
    assets = []
    for path, kind, name in _resolve_assets(args):
        assets.append(
            {
                "path": path,
                "kind": kind,
                "name": name,
                "trusted": bool(args.asset_trusted),
                "remote": bool(args.allow_remote_assets),
            }
        )

    output_path = getattr(args, "out", None)
    if output_path is None:
        output_path = getattr(args, "emit_pdf", None)
    watermark_layer = _normalize_watermark_layer(getattr(args, "watermark_layer", "overlay"))
    manifest = {
        "schema": "fullbleed.compiler_input.v1",
        "command": command_name,
        "html": {"path": getattr(args, "html", None), "inline": getattr(args, "html_str", None)},
        "css": {"paths": getattr(args, "css", None) or [], "inline": getattr(args, "css_str", None) or []},
        "assets": assets,
        "page": {
            "size": getattr(args, "page_size", None),
            "width": getattr(args, "page_width", None),
            "height": getattr(args, "page_height", None),
            "margin": getattr(args, "margin", None),
            "page_margins": getattr(args, "page_margins", None),
            "resolved_width": page_width,
            "resolved_height": page_height,
        },
        "engine": {
            "reuse_xobjects": getattr(args, "reuse_xobjects", None),
            "svg_form_xobjects": getattr(args, "svg_form_xobjects", None),
            "svg_raster_fallback": getattr(args, "svg_raster_fallback", None),
            "unicode_support": getattr(args, "unicode_support", None),
            "shape_text": getattr(args, "shape_text", None),
            "unicode_metrics": getattr(args, "unicode_metrics", None),
            "jit_mode": getattr(args, "jit_mode", None),
        },
        "watermark": {
            "text": getattr(args, "watermark_text", None),
            "html": getattr(args, "watermark_html", None),
            "image": getattr(args, "watermark_image", None),
            "layer": watermark_layer,
            "semantics": getattr(args, "watermark_semantics", "artifact"),
            "opacity": getattr(args, "watermark_opacity", None),
            "rotation": getattr(args, "watermark_rotation", None),
            "enabled": bool(
                getattr(args, "watermark_text", None)
                or getattr(args, "watermark_html", None)
                or getattr(args, "watermark_image", None)
            ),
        },
        "pdf": {
            "version": getattr(args, "pdf_version", None),
            "profile": getattr(args, "pdf_profile", None),
            "output_intent_icc": getattr(args, "output_intent_icc", None),
            "output_intent_identifier": getattr(args, "output_intent_identifier", None),
            "output_intent_info": getattr(args, "output_intent_info", None),
            "output_intent_components": getattr(args, "output_intent_components", None),
            "color_space": getattr(args, "color_space", None),
            "document_lang": getattr(args, "document_lang", None),
            "document_title": getattr(args, "document_title", None),
        },
        "profile": getattr(args, "profile", None),
        "fail_on": getattr(args, "fail_on", None) or [],
        "allow_fallbacks": bool(getattr(args, "allow_fallbacks", False)),
        "budget": {
            "max_pages": getattr(args, "budget_max_pages", None),
            "max_bytes": getattr(args, "budget_max_bytes", None),
            "max_ms": getattr(args, "budget_max_ms", None),
        },
        "repro": {
            "record": getattr(args, "repro_record", None),
            "check": getattr(args, "repro_check", None),
        },
        "artifacts": {
            "jit": getattr(args, "emit_jit", None),
            "perf": getattr(args, "emit_perf", None),
            "glyph_report": getattr(args, "emit_glyph_report", None),
            "page_data": getattr(args, "emit_page_data", None),
            "image": getattr(args, "emit_image", None),
            "image_dpi": getattr(args, "image_dpi", 150),
        },
        "output": {
            "path": output_path,
            "stdout": output_path == "-",
            "deterministic_hash": getattr(args, "deterministic_hash", None),
        },
    }
    if getattr(args, "entrypoint", None):
        manifest["entrypoint"] = args.entrypoint
    return manifest


def _build_engine(args):
    """Construct `PdfEngine` from normalized CLI options."""
    # Apply profile defaults before building engine
    _apply_profile(args)
    
    page_width, page_height = _resolve_page_size(args)
    page_margins = _read_json_or_path(args.page_margins)
    
    # Profile may enable jit/perf logging
    emit_jit = args.emit_jit
    emit_perf = args.emit_perf
    
    # Preflight profile auto-enables jit logging if not specified
    profile = getattr(args, "profile", None)
    if profile and profile.lower() == "preflight":
        if not emit_jit:
            emit_jit = "fullbleed_preflight.jit"
        if not emit_perf:
            emit_perf = "fullbleed_preflight.perf"
    
    watermark_layer = _normalize_watermark_layer(getattr(args, "watermark_layer", "overlay"))
    try:
        engine = fullbleed.PdfEngine(
            page_width=page_width,
            page_height=page_height,
            margin=args.margin,
            page_margins=page_margins,
            reuse_xobjects=args.reuse_xobjects,
            svg_form_xobjects=args.svg_form_xobjects,
            svg_raster_fallback=args.svg_raster_fallback,
            unicode_support=args.unicode_support,
            shape_text=args.shape_text,
            unicode_metrics=args.unicode_metrics,
            pdf_version=args.pdf_version,
            pdf_profile=args.pdf_profile,
            output_intent_icc=getattr(args, "output_intent_icc", None),
            output_intent_identifier=getattr(args, "output_intent_identifier", None),
            output_intent_info=getattr(args, "output_intent_info", None),
            output_intent_components=getattr(args, "output_intent_components", None),
            color_space=args.color_space,
            document_lang=args.document_lang,
            document_title=args.document_title,
            header_each=getattr(args, "header_each", None),
            footer_each=getattr(args, "footer_each", None),
            watermark_text=args.watermark_text,
            watermark_html=args.watermark_html,
            watermark_image=args.watermark_image,
            watermark_layer=watermark_layer,
            watermark_semantics=getattr(args, "watermark_semantics", "artifact"),
            watermark_opacity=args.watermark_opacity,
            watermark_rotation=args.watermark_rotation,
            jit_mode=args.jit_mode,
            debug=bool(emit_jit),
            debug_out=emit_jit,
            perf=bool(emit_perf),
            perf_out=emit_perf,
        )
    except Exception as exc:
        message = str(exc)
        if "pdfx4 requires embedded fonts" in message.lower():
            raise ValueError(
                message
                + " Hint: add an embeddable font asset (for example '--asset @noto-sans') "
                "and set CSS 'font-family' to that font."
            )
        raise
    if args.asset:
        bundle = _build_bundle(args)
        engine.register_bundle(bundle)
    return engine


def _write_pdf_bytes(out_path, pdf_bytes):
    """Write PDF bytes to file or stdout and return written byte count."""
    if out_path == "-":
        sys.stdout.buffer.write(pdf_bytes)
        return len(pdf_bytes)
    Path(out_path).write_bytes(pdf_bytes)
    return len(pdf_bytes)


def _derive_image_stem(out_path):
    """Derive PNG artifact filename stem from output PDF path."""
    if out_path and out_path != "-":
        stem = Path(out_path).stem
        if stem:
            return stem
    return "render"


def _emit_image_artifacts(engine, html, css, out_path, args):
    """Render per-page PNG artifacts when `--emit-image` is set."""
    image_dir = getattr(args, "emit_image", None)
    if not image_dir:
        return None

    raw_dpi = getattr(args, "image_dpi", 150)
    try:
        dpi = int(raw_dpi)
    except (TypeError, ValueError):
        raise ValueError(f"--image-dpi must be a positive integer (got {raw_dpi!r})")
    if dpi <= 0:
        raise ValueError(f"--image-dpi must be a positive integer (got {dpi})")

    out_dir = Path(image_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    stem = _derive_image_stem(out_path)

    if hasattr(engine, "render_image_pages_to_dir"):
        paths = engine.render_image_pages_to_dir(html, css, str(out_dir), dpi, stem)
        return [str(p) for p in (paths or [])]

    if not hasattr(engine, "render_image_pages"):
        raise ValueError(
            "installed engine does not support image artifacts (missing render_image_pages API)"
        )

    page_images = engine.render_image_pages(html, css, dpi)
    paths = []
    for idx0, png_bytes in enumerate(page_images, start=1):
        path = out_dir / f"{stem}_page{idx0}.png"
        path.write_bytes(png_bytes)
        paths.append(str(path))
    return paths


def _json_default(obj):
    """Best-effort JSON serializer fallback for CLI payload objects."""
    if isinstance(obj, (bytes, bytearray)):
        return {"type": "bytes", "length": len(obj)}
    if isinstance(obj, memoryview):
        return {"type": "bytes", "length": len(obj.tobytes())}
    if isinstance(obj, Path):
        return str(obj)
    return str(obj)


def _json_dumps(payload, indent=None):
    """JSON serialize payload using CLI defaults."""
    return json.dumps(payload, ensure_ascii=True, indent=indent, default=_json_default)


def _write_json(path, payload):
    """Write payload as pretty JSON with stable CLI serializer rules."""
    Path(path).write_text(_json_dumps(payload, indent=2), encoding="utf-8")


def _extract_pdf_bytes(payload):
    """Recursively locate PDF byte payload in mixed return structures."""
    if isinstance(payload, (bytes, bytearray)):
        return bytes(payload)
    if isinstance(payload, memoryview):
        return payload.tobytes()
    if isinstance(payload, (list, tuple)):
        for item in payload:
            resolved = _extract_pdf_bytes(item)
            if resolved is not None:
                return resolved
    return None


def _normalize_bytes_written(raw, pdf_bytes=None):
    """Normalize render return values to an integer byte count."""
    if isinstance(raw, bool):
        return int(raw)
    if isinstance(raw, int):
        return raw
    if isinstance(raw, float):
        return int(raw)
    if isinstance(raw, str):
        try:
            return int(raw)
        except ValueError:
            pass
    if isinstance(raw, (bytes, bytearray, memoryview)):
        return len(bytes(raw))
    if isinstance(raw, (list, tuple)):
        for item in raw:
            if isinstance(item, bool):
                continue
            if isinstance(item, int):
                return item
            if isinstance(item, float):
                return int(item)
        embedded = _extract_pdf_bytes(raw)
        if embedded is not None:
            return len(embedded)
    embedded = _extract_pdf_bytes(pdf_bytes)
    if embedded is not None:
        return len(embedded)
    return 0


def _sha256_bytes(data):
    """Return SHA-256 digest for raw bytes."""
    return hashlib.sha256(data).hexdigest()


def _stable_json_hash(payload):
    """Return stable SHA-256 hash of normalized JSON payload."""
    normalized = json.dumps(
        payload,
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
        default=_json_default,
    )
    return _sha256_bytes(normalized.encode("utf-8"))


def _compute_pdf_sha256(out_path, pdf_bytes):
    """Compute output PDF SHA-256 from file path or in-memory bytes."""
    output_hash = None
    if out_path and out_path != "-":
        pdf_path = Path(out_path)
        if pdf_path.exists():
            output_hash = _sha256_bytes(pdf_path.read_bytes())
    if output_hash is None:
        resolved_pdf = _extract_pdf_bytes(pdf_bytes)
        if resolved_pdf is not None:
            output_hash = _sha256_bytes(resolved_pdf)
    return output_hash


def _compute_output_hash(deterministic_hash, out_path, pdf_bytes):
    """Compute and optionally write deterministic output hash artifact."""
    output_hash = _compute_pdf_sha256(out_path, pdf_bytes)
    if deterministic_hash and output_hash is not None:
        Path(deterministic_hash).write_text(output_hash, encoding="utf-8")
    return output_hash


def _assets_lock_hash():
    """Return digest metadata for `assets.lock.json` when present."""
    lock = Path("assets.lock.json")
    if not lock.exists():
        return None
    return {
        "path": str(lock),
        "sha256": _sha256_bytes(lock.read_bytes()),
    }


def _build_repro_record(args, manifest, html, css, output_hash, bytes_written):
    """Create a reproducibility record for the current render invocation."""
    assets = []
    for path, kind, name in _resolve_assets(args):
        item = {
            "kind": kind,
            "name": name,
            "source": path,
            "trusted": bool(getattr(args, "asset_trusted", False)),
            "remote": bool(getattr(args, "allow_remote_assets", False)),
        }
        if not str(path).startswith(("http://", "https://")):
            p = Path(path)
            if p.exists() and p.is_file():
                item["sha256"] = _sha256_bytes(p.read_bytes())
                item["size_bytes"] = int(p.stat().st_size)
        assets.append(item)
    assets.sort(key=lambda row: (row.get("kind", ""), row.get("name") or "", row.get("source", "")))

    input_fingerprint = {
        "command": manifest.get("command"),
        "html_sha256": _sha256_bytes(html.encode("utf-8")),
        "css_sha256": _sha256_bytes(css.encode("utf-8")),
        "assets": assets,
        "page": manifest.get("page"),
        "engine": manifest.get("engine"),
        "pdf": manifest.get("pdf"),
        "profile": manifest.get("profile"),
        "fail_on": sorted(manifest.get("fail_on", [])),
        "budget": manifest.get("budget", {}),
        "allow_fallbacks": bool(manifest.get("allow_fallbacks", False)),
    }
    return {
        "schema": "fullbleed.repro_record.v1",
        "created_at": datetime.now(timezone.utc).isoformat(),
        "cli_version": _get_version(),
        "python": sys.version.split()[0],
        "platform": sys.platform,
        "input_manifest_sha256": _stable_json_hash(manifest),
        "input_fingerprint_sha256": _stable_json_hash(input_fingerprint),
        "output_pdf_sha256": output_hash,
        "output_bytes_written": int(_normalize_bytes_written(bytes_written)),
        "assets_lock": _assets_lock_hash(),
    }


def _render_with_artifacts(engine, html, css, out_path, args):
    """Render PDF and optional JSON/PNG artifacts, returning tuple payload."""
    page_data_path = args.emit_page_data
    glyph_path = args.emit_glyph_report
    
    # For fail-on checks, we need glyph report even if not explicitly requested
    fail_on = getattr(args, "fail_on", None) or []
    need_glyph_check = "missing-glyphs" in fail_on or "font-subst" in fail_on
    
    if page_data_path and glyph_path:
        if not getattr(args, "json_only", False):
            sys.stderr.write(
                "[warn] both --emit-page-data and --emit-glyph-report set; rendering twice\n"
            )
        primary_pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        _write_json(page_data_path, page_data)
        bytes_written = _write_pdf_bytes(out_path, primary_pdf_bytes)
        _glyph_pdf_bytes, glyph = engine.render_pdf_with_glyph_report(html, css)
        _write_json(glyph_path, glyph)
        image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
        return bytes_written, glyph, primary_pdf_bytes, image_paths

    if page_data_path:
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        _write_json(page_data_path, page_data)
        bytes_written = _write_pdf_bytes(out_path, pdf_bytes)
        # If we need glyph check, render again for glyph report
        if need_glyph_check:
            _glyph_pdf_bytes, glyph = engine.render_pdf_with_glyph_report(html, css)
            image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
            return bytes_written, glyph, pdf_bytes, image_paths
        image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
        return bytes_written, None, pdf_bytes, image_paths
    
    if glyph_path:
        pdf_bytes, glyph = engine.render_pdf_with_glyph_report(html, css)
        _write_json(glyph_path, glyph)
        bytes_written = _write_pdf_bytes(out_path, pdf_bytes)
        image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
        return bytes_written, glyph, pdf_bytes, image_paths

    if out_path == "-":
        pdf_bytes = engine.render_pdf(html, css)
        bytes_written = _write_pdf_bytes(out_path, pdf_bytes)
        image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
        return bytes_written, None, pdf_bytes, image_paths
    
    # For fail-on checks, we need glyph report even if not explicitly requested
    fail_on = getattr(args, "fail_on", None) or []
    need_glyph_check = "missing-glyphs" in fail_on or "font-subst" in fail_on
    
    if need_glyph_check:
        pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report(html, css)
        bytes_written = _write_pdf_bytes(out_path, pdf_bytes)
        image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
        return bytes_written, glyph_report, pdf_bytes, image_paths
    
    bytes_written = engine.render_pdf_to_file(html, css, out_path)
    image_paths = _emit_image_artifacts(engine, html, css, out_path, args)
    return _normalize_bytes_written(bytes_written), None, None, image_paths


def _as_float(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _as_int(value):
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _prepare_fail_on_args(args):
    """Inject temporary artifacts/options required by active fail-on checks."""
    fail_on = set(getattr(args, "fail_on", None) or [])
    previous = {}
    internal_jit_path = None

    needs_jit = "overflow" in fail_on or "font-subst" in fail_on
    if "budget" in fail_on:
        if getattr(args, "budget_max_pages", None) is not None:
            needs_jit = True
        if getattr(args, "budget_max_ms", None) is not None:
            needs_jit = True

    if needs_jit and not getattr(args, "emit_jit", None):
        handle, temp_path = tempfile.mkstemp(prefix="fullbleed_failon_", suffix=".jit.jsonl")
        os.close(handle)
        previous["emit_jit"] = getattr(args, "emit_jit", None)
        args.emit_jit = temp_path
        internal_jit_path = temp_path

    if "overflow" in fail_on and not getattr(args, "jit_mode", None):
        previous["jit_mode"] = getattr(args, "jit_mode", None)
        args.jit_mode = "plan"

    return previous, internal_jit_path


def _restore_args(args, previous):
    """Restore argparse fields modified during render/verify pre-processing."""
    for key, value in previous.items():
        setattr(args, key, value)


def _collect_jit_insights(jit_log_path):
    """Collect overflow, fallback, and timing signals from JIT diagnostics."""
    insights = {
        "overflow_signal": False,
        "overflow_count": 0,
        "overflow_samples": [],
        "debug_counts": {},
        "winansi_fallbacks": 0,
        "pages": None,
        "total_ms": None,
    }

    if not jit_log_path:
        return insights
    path = Path(jit_log_path)
    if not path.exists():
        return insights

    for entry in _load_json_lines(path):
        if not isinstance(entry, dict):
            continue
        entry_type = entry.get("type")
        if entry_type == "debug.summary":
            counts = entry.get("counts", {})
            if isinstance(counts, dict):
                for key, value in counts.items():
                    ivalue = _as_int(value)
                    if ivalue is None:
                        continue
                    insights["debug_counts"][key] = insights["debug_counts"].get(key, 0) + ivalue
        elif entry_type == "pdf.winansi.fallback":
            ivalue = _as_int(entry.get("fallbacks"))
            if ivalue:
                insights["winansi_fallbacks"] += ivalue
        elif entry_type == "jit.metrics":
            counts = entry.get("counts", {})
            if isinstance(counts, dict):
                page_count = _as_int(counts.get("pages"))
                if page_count is not None:
                    insights["pages"] = page_count
            timings = entry.get("timing_ms", {})
            if isinstance(timings, dict):
                total = 0.0
                any_timing = False
                for value in timings.values():
                    fvalue = _as_float(value)
                    if fvalue is None:
                        continue
                    total += fvalue
                    any_timing = True
                if any_timing:
                    insights["total_ms"] = total
        elif entry_type == "jit.docplan":
            page_size = entry.get("page_size", {})
            if not isinstance(page_size, dict):
                continue
            page_w = _as_float(page_size.get("w"))
            page_h = _as_float(page_size.get("h"))
            if page_w is None or page_h is None:
                continue
            insights["overflow_signal"] = True
            for page in entry.get("pages", []) or []:
                if not isinstance(page, dict):
                    continue
                page_number = page.get("n")
                for placement in page.get("placements", []) or []:
                    if not isinstance(placement, dict):
                        continue
                    bbox = placement.get("bbox")
                    if not isinstance(bbox, dict):
                        continue
                    x = _as_float(bbox.get("x"))
                    y = _as_float(bbox.get("y"))
                    w = _as_float(bbox.get("w"))
                    h = _as_float(bbox.get("h"))
                    if x is None or y is None or w is None or h is None:
                        continue
                    if x < -0.01 or y < -0.01 or (x + w) > (page_w + 0.01) or (y + h) > (page_h + 0.01):
                        insights["overflow_count"] += 1
                        if len(insights["overflow_samples"]) < 5:
                            insights["overflow_samples"].append(
                                {
                                    "page": page_number,
                                    "bbox": {"x": x, "y": y, "w": w, "h": h},
                                    "page_size": {"w": page_w, "h": page_h},
                                }
                            )
    return insights


def _validate_budget_configuration(args):
    """Ensure budget fail-on has at least one configured budget threshold."""
    fail_on = set(getattr(args, "fail_on", None) or [])
    if "budget" not in fail_on:
        return
    max_pages = getattr(args, "budget_max_pages", None)
    max_bytes = getattr(args, "budget_max_bytes", None)
    max_ms = getattr(args, "budget_max_ms", None)
    if max_pages is None and max_bytes is None and max_ms is None:
        raise ValueError(
            "--fail-on budget requires at least one limit: --budget-max-pages, --budget-max-bytes, or --budget-max-ms"
        )


def _evaluate_failures(args, bytes_written, glyph_report, jit_insights):
    """Evaluate fail-on policy and return structured violation objects."""
    fail_on = set(getattr(args, "fail_on", None) or [])
    allow_fallbacks = bool(getattr(args, "allow_fallbacks", False))
    failures = []

    missing_count = len(glyph_report or [])
    if "missing-glyphs" in fail_on and missing_count > 0 and not allow_fallbacks:
        failures.append(
            {
                "code": "MISSING_GLYPHS",
                "message": f"Missing glyphs detected: {missing_count} unique codepoints",
                "count": missing_count,
            }
        )

    if "font-subst" in fail_on:
        summary_counts = jit_insights.get("debug_counts", {})
        winansi_fallbacks = (jit_insights.get("winansi_fallbacks", 0) or 0) + (
            _as_int(summary_counts.get("pdf.winansi.fallback")) or 0
        )
        winansi_lossy = _as_int(summary_counts.get("pdf.winansi.lossy")) or 0
        font_subst_count = winansi_fallbacks + winansi_lossy
        if (missing_count > 0 or font_subst_count > 0) and not allow_fallbacks:
            failures.append(
                {
                    "code": "FONT_SUBSTITUTION_DETECTED",
                    "message": "Font substitution fallback/lossy encoding signals were detected",
                    "missing_glyphs": missing_count,
                    "encoding_fallbacks": font_subst_count,
                }
            )

    if "overflow" in fail_on:
        if not jit_insights.get("overflow_signal"):
            failures.append(
                {
                    "code": "OVERFLOW_SIGNAL_UNAVAILABLE",
                    "message": "Overflow check requested but no jit.docplan signal was emitted",
                }
            )
        elif jit_insights.get("overflow_count", 0) > 0:
            failures.append(
                {
                    "code": "OVERFLOW_DETECTED",
                    "message": f"Placement bounds exceeded page size ({jit_insights['overflow_count']} placement(s))",
                    "count": jit_insights["overflow_count"],
                    "samples": jit_insights.get("overflow_samples", []),
                }
            )

    if "budget" in fail_on:
        observed_bytes = int(_normalize_bytes_written(bytes_written))
        max_bytes = getattr(args, "budget_max_bytes", None)
        if max_bytes is not None and observed_bytes > max_bytes:
            failures.append(
                {
                    "code": "BUDGET_BYTES_EXCEEDED",
                    "message": f"PDF size {observed_bytes} exceeded budget {max_bytes}",
                    "observed": observed_bytes,
                    "budget": max_bytes,
                }
            )

        max_pages = getattr(args, "budget_max_pages", None)
        observed_pages = jit_insights.get("pages")
        if max_pages is not None:
            if observed_pages is None:
                failures.append(
                    {
                        "code": "BUDGET_SIGNAL_UNAVAILABLE",
                        "message": "Page-count budget requested but jit metrics were unavailable",
                        "metric": "pages",
                    }
                )
            elif observed_pages > max_pages:
                failures.append(
                    {
                        "code": "BUDGET_PAGES_EXCEEDED",
                        "message": f"Page count {observed_pages} exceeded budget {max_pages}",
                        "observed": observed_pages,
                        "budget": max_pages,
                    }
                )

        max_ms = getattr(args, "budget_max_ms", None)
        observed_ms = jit_insights.get("total_ms")
        if max_ms is not None:
            if observed_ms is None:
                failures.append(
                    {
                        "code": "BUDGET_SIGNAL_UNAVAILABLE",
                        "message": "Timing budget requested but jit metrics were unavailable",
                        "metric": "timing_ms",
                    }
                )
            elif observed_ms > max_ms:
                failures.append(
                    {
                        "code": "BUDGET_TIME_EXCEEDED",
                        "message": f"Render timing {observed_ms:.3f}ms exceeded budget {max_ms:.3f}ms",
                        "observed": observed_ms,
                        "budget": max_ms,
                    }
                )

    return failures


def _collect_fallback_summary(args, glyph_report, jit_insights):
    """Summarize fallback-related signals for result payload emission."""
    summary_counts = jit_insights.get("debug_counts", {})
    winansi_fallbacks = (jit_insights.get("winansi_fallbacks", 0) or 0) + (
        _as_int(summary_counts.get("pdf.winansi.fallback")) or 0
    )
    winansi_lossy = _as_int(summary_counts.get("pdf.winansi.lossy")) or 0
    missing_glyphs = len(glyph_report or [])
    encoding_fallbacks = winansi_fallbacks + winansi_lossy
    return {
        "allowed": bool(getattr(args, "allow_fallbacks", False)),
        "missing_glyphs": missing_glyphs,
        "encoding_fallbacks": encoding_fallbacks,
        "detected": (missing_glyphs + encoding_fallbacks) > 0,
    }


def _run_repro_record_or_check(args, manifest, html, css, output_hash, bytes_written):
    """Write/check reproducibility records and return associated failures."""
    repro_record_path = getattr(args, "repro_record", None)
    repro_check_path = getattr(args, "repro_check", None)
    if not repro_record_path and not repro_check_path:
        return None, []

    current_record = _build_repro_record(args, manifest, html, css, output_hash, bytes_written)
    failures = []

    if repro_record_path:
        Path(repro_record_path).write_text(
            _json_dumps(current_record, indent=2),
            encoding="utf-8",
        )

    if repro_check_path:
        check_file = Path(repro_check_path)
        if not check_file.exists():
            failures.append(
                {
                    "code": "REPRO_RECORD_NOT_FOUND",
                    "message": f"Repro check file not found: {repro_check_path}",
                }
            )
        else:
            try:
                expected = json.loads(check_file.read_text(encoding="utf-8"))
            except json.JSONDecodeError as exc:
                failures.append(
                    {
                        "code": "REPRO_RECORD_INVALID",
                        "message": f"Failed to parse repro record JSON: {exc}",
                    }
                )
                expected = {}
            expected_input = expected.get("input_fingerprint_sha256")
            expected_output = expected.get("output_pdf_sha256")
            if expected_input and expected_input != current_record["input_fingerprint_sha256"]:
                failures.append(
                    {
                        "code": "REPRO_INPUT_DRIFT",
                        "message": "Input fingerprint drift detected compared to repro record",
                        "expected": expected_input,
                        "observed": current_record["input_fingerprint_sha256"],
                    }
                )
            if expected_output and expected_output != current_record["output_pdf_sha256"]:
                failures.append(
                    {
                        "code": "REPRO_HASH_MISMATCH",
                        "message": "Output PDF hash mismatch against repro record",
                        "expected": expected_output,
                        "observed": current_record["output_pdf_sha256"],
                    }
                )
            expected_lock = (expected.get("assets_lock") or {}).get("sha256")
            observed_lock = (current_record.get("assets_lock") or {}).get("sha256")
            if expected_lock and observed_lock and expected_lock != observed_lock:
                failures.append(
                    {
                        "code": "REPRO_LOCK_MISMATCH",
                        "message": "assets.lock.json hash mismatch against repro record",
                        "expected": expected_lock,
                        "observed": observed_lock,
                    }
                )

    return current_record, failures


def _emit_result(ok, schema, out_path, bytes_written, outputs, args, error=None):
    """Emit command result in human-readable or schema-based JSON format."""
    if not args.json:
        if ok:
            msg = f"[ok] wrote {out_path} ({_normalize_bytes_written(bytes_written)} bytes)"
            sys.stdout.write(msg + "\n")
            image_paths = []
            image_dir = None
            if isinstance(outputs, dict):
                image_paths = outputs.get("image_paths") or []
                image_dir = outputs.get("image")
            if image_paths:
                if image_dir:
                    sys.stdout.write(
                        f"[ok] wrote {len(image_paths)} PNG page(s) -> {image_dir}\n"
                    )
                else:
                    sys.stdout.write(f"[ok] wrote {len(image_paths)} PNG page(s)\n")
            return
        else:
            msg = f"[error] {error.get('code')}: {error.get('message')}"
        sys.stdout.write(msg + "\n")
        return
    payload = {"schema": schema, "ok": ok, "outputs": outputs}
    if ok:
        payload["bytes_written"] = _normalize_bytes_written(bytes_written)
    else:
        payload.update(error or {})
    sys.stdout.write(_json_dumps(payload) + "\n")


def cmd_render(args):
    """CLI handler for `fullbleed render`."""
    html = _collect_html(args)
    css = _collect_css(args)
    _validate_pdf_options(args)
    _validate_budget_configuration(args)
    if args.json and args.out == "-":
        raise ValueError("--json cannot be used with --out - (stdout PDF)")
    remote_refs = _detect_remote_refs(html, css)
    if remote_refs and not args.allow_remote_assets and not getattr(args, "json_only", False):
        sys.stderr.write(
            "[warn] remote asset refs detected; use --asset and --allow-remote-assets if needed\n"
        )

    user_emit_jit = getattr(args, "emit_jit", None)
    previous, internal_jit_path = _prepare_fail_on_args(args)
    try:
        engine = _build_engine(args)
        rendered = _render_with_artifacts(engine, html, css, args.out, args)
    finally:
        _restore_args(args, previous)
    if len(rendered) == 3:
        bytes_written, glyph_report, pdf_bytes = rendered
        image_paths = None
    else:
        bytes_written, glyph_report, pdf_bytes, image_paths = rendered
    # Flush/close debug logger buffers before reading emitted jit logs.
    try:
        del engine
    except UnboundLocalError:
        pass

    deterministic_hash = getattr(args, "deterministic_hash", None)
    output_hash = _compute_output_hash(deterministic_hash, args.out, pdf_bytes)

    effective_jit_path = user_emit_jit or internal_jit_path
    jit_insights = _collect_jit_insights(effective_jit_path)
    failures = _evaluate_failures(args, bytes_written, glyph_report, jit_insights)
    fallback_summary = _collect_fallback_summary(args, glyph_report, jit_insights)

    manifest = _build_manifest(args)
    repro_record, repro_failures = _run_repro_record_or_check(
        args,
        manifest,
        html,
        css,
        output_hash,
        bytes_written,
    )
    failures.extend(repro_failures)

    if internal_jit_path:
        try:
            Path(internal_jit_path).unlink(missing_ok=True)
        except Exception:
            pass

    outputs = {
        "pdf": None if args.out == "-" else args.out,
        "jit": user_emit_jit,
        "perf": args.emit_perf,
        "glyph_report": args.emit_glyph_report,
        "page_data": args.emit_page_data,
        "image": getattr(args, "emit_image", None),
        "image_paths": image_paths,
        "deterministic_hash": deterministic_hash,
        "sha256": output_hash,
        "fallbacks": fallback_summary,
        "repro_record": getattr(args, "repro_record", None),
        "repro_check": getattr(args, "repro_check", None),
        "repro_status": "fail" if failures else ("pass" if repro_record else None),
    }
    
    if failures:
        error = {
            "code": failures[0]["code"],
            "message": failures[0]["message"],
            "failures": failures,
        }
        _emit_result(False, "fullbleed.render_result.v1", args.out, bytes_written, outputs, args, error=error)
        raise SystemExit(1)
    
    _emit_result(True, "fullbleed.render_result.v1", args.out, bytes_written, outputs, args)


def cmd_verify(args):
    """CLI handler for `fullbleed verify`."""
    html = _collect_html(args)
    css = _collect_css(args)
    _validate_pdf_options(args)
    _validate_budget_configuration(args)
    if args.emit_pdf and args.json and args.emit_pdf == "-":
        raise ValueError("--json cannot be used with --emit-pdf - (stdout PDF)")
    remote_refs = _detect_remote_refs(html, css)
    if remote_refs and not args.allow_remote_assets and not getattr(args, "json_only", False):
        sys.stderr.write(
            "[warn] remote asset refs detected; use --asset and --allow-remote-assets if needed\n"
        )
    user_emit_jit = getattr(args, "emit_jit", None)
    previous, internal_jit_path = _prepare_fail_on_args(args)
    if user_emit_jit and not args.emit_pdf and not args.jit_mode:
        previous.setdefault("jit_mode", getattr(args, "jit_mode", None))
        args.jit_mode = "plan"
    out_path = args.emit_pdf or "-"
    try:
        engine = _build_engine(args)
        rendered = _render_with_artifacts(engine, html, css, out_path, args)
    finally:
        _restore_args(args, previous)
    if len(rendered) == 3:
        bytes_written, glyph_report, pdf_bytes = rendered
        image_paths = None
    else:
        bytes_written, glyph_report, pdf_bytes, image_paths = rendered
    # Flush/close debug logger buffers before reading emitted jit logs.
    try:
        del engine
    except UnboundLocalError:
        pass
    deterministic_hash = getattr(args, "deterministic_hash", None)
    output_hash = _compute_output_hash(deterministic_hash, out_path, pdf_bytes)

    effective_jit_path = user_emit_jit or internal_jit_path
    jit_insights = _collect_jit_insights(effective_jit_path)
    failures = _evaluate_failures(args, bytes_written, glyph_report, jit_insights)
    fallback_summary = _collect_fallback_summary(args, glyph_report, jit_insights)

    manifest = _build_manifest(args)
    repro_record, repro_failures = _run_repro_record_or_check(
        args,
        manifest,
        html,
        css,
        output_hash,
        bytes_written,
    )
    failures.extend(repro_failures)

    if internal_jit_path:
        try:
            Path(internal_jit_path).unlink(missing_ok=True)
        except Exception:
            pass

    outputs = {
        "pdf": None if out_path == "-" else out_path,
        "jit": user_emit_jit,
        "perf": args.emit_perf,
        "glyph_report": args.emit_glyph_report,
        "page_data": args.emit_page_data,
        "image": getattr(args, "emit_image", None),
        "image_paths": image_paths,
        "deterministic_hash": deterministic_hash,
        "sha256": output_hash,
        "fallbacks": fallback_summary,
        "repro_record": getattr(args, "repro_record", None),
        "repro_check": getattr(args, "repro_check", None),
        "repro_status": "fail" if failures else ("pass" if repro_record else None),
    }
    if failures:
        error = {
            "code": failures[0]["code"],
            "message": failures[0]["message"],
            "failures": failures,
        }
        _emit_result(False, "fullbleed.verify_result.v1", out_path, bytes_written, outputs, args, error=error)
        raise SystemExit(1)
    _emit_result(True, "fullbleed.verify_result.v1", out_path, bytes_written, outputs, args)


def cmd_plan(args):
    """CLI handler for `fullbleed plan`."""
    html = _collect_html(args)
    css = _collect_css(args)
    remote_refs = _detect_remote_refs(html, css)
    warnings = []
    if remote_refs and not args.allow_remote_assets:
        warnings.append(
            {
                "code": "REMOTE_REFS_DETECTED",
                "message": "Remote asset refs detected; use --asset and --allow-remote-assets if needed",
                "count": len(remote_refs),
                "refs": remote_refs,
            }
        )
        if not args.json and not getattr(args, "json_only", False):
            sys.stderr.write(
                "[warn] remote asset refs detected; use --asset and --allow-remote-assets if needed\n"
            )
    manifest = _build_manifest(args)
    payload = {
        "schema": "fullbleed.plan_result.v1",
        "ok": True,
        "manifest": manifest,
        "warnings": warnings,
    }
    if args.json:
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write("[ok] plan compiled\n")
        if warnings:
            sys.stdout.write(f"[warn] {len(warnings)} warning(s)\n")


def _load_json_lines(path):
    """Read newline-delimited JSON entries from a file."""
    data = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            data.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return data


def cmd_debug_perf(args):
    """CLI handler for `fullbleed debug-perf`."""
    data = _load_json_lines(args.perf_log)
    hot = [d for d in data if d.get("type", "").startswith("perf.hot.")]
    if hot:
        out = hot
    else:
        spans = {}
        for entry in data:
            if entry.get("type") != "perf.span":
                continue
            name = entry.get("name")
            spans[name] = spans.get(name, 0.0) + float(entry.get("ms", 0.0))
        out = [{"name": k, "ms": v} for k, v in sorted(spans.items(), key=lambda x: -x[1])]
    if args.json:
        sys.stdout.write(json.dumps(out[: args.top], ensure_ascii=True) + "\n")
    else:
        for entry in out[: args.top]:
            if "name" in entry:
                sys.stdout.write(f"{entry['name']}: {entry.get('ms', entry.get('value', ''))}\n")


def cmd_debug_jit(args):
    """CLI handler for `fullbleed debug-jit`."""
    data = _load_json_lines(args.jit_log)
    if args.errors_only:
        data = [
            d
            for d in data
            if "error" in d.get("type", "").lower()
            or d.get("level") in {"error", "warn"}
        ]
    if args.json:
        sys.stdout.write(json.dumps(data, ensure_ascii=True) + "\n")
    else:
        for entry in data:
            sys.stdout.write(json.dumps(entry, ensure_ascii=True) + "\n")


def cmd_doctor(args):
    """CLI handler for `fullbleed doctor` environment diagnostics."""
    bootstrap = Path(fullbleed_assets.asset_path("bootstrap.min.css"))
    bootstrap_icons = Path(fullbleed_assets.asset_path("icons/bootstrap-icons.svg"))
    noto = Path(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))
    checks = [
        {"name": "python>=3.8", "ok": sys.version_info >= (3, 8), "detail": sys.version.split()[0]},
        {"name": "fullbleed.PdfEngine", "ok": hasattr(fullbleed, "PdfEngine"), "detail": "available"},
        {"name": "fullbleed.AssetBundle", "ok": hasattr(fullbleed, "AssetBundle"), "detail": "available"},
        {"name": "assets.bootstrap", "ok": bootstrap.exists(), "detail": str(bootstrap)},
        {"name": "assets.bootstrap_icons", "ok": bootstrap_icons.exists(), "detail": str(bootstrap_icons)},
        {"name": "assets.noto_sans", "ok": noto.exists(), "detail": str(noto)},
    ]
    ok = all(c["ok"] for c in checks)
    report = {
        "schema": "fullbleed.doctor.v1",
        "ok": ok,
        "python": sys.version.split()[0],
        "platform": sys.platform,
        "pdf_versions": ["1.7", "2.0"],
        "pdf_profiles": ["none", "pdfa2b", "pdfx4", "tagged"],
        "color_spaces": ["rgb", "cmyk"],
        "assets": {
            "bootstrap": str(bootstrap),
            "bootstrap_icons": str(bootstrap_icons),
            "noto_sans": str(noto),
        },
        "checks": checks,
    }
    if args.json:
        sys.stdout.write(json.dumps(report, ensure_ascii=True) + "\n")
    else:
        for k, v in report.items():
            sys.stdout.write(f"{k}: {v}\n")
    if getattr(args, "strict", False) and not ok:
        raise SystemExit(3)


def cmd_compliance(args):
    """CLI handler for compliance and license policy reporting."""
    now = datetime.now(timezone.utc)
    max_age_days = max(int(getattr(args, "max_audit_age_days", 180) or 180), 1)
    license_state = _resolve_license_attestation(args)
    commercial_attested = (
        license_state["mode"] == "commercial" and license_state["attested"]
    )

    license_file = _find_compliance_file(COMPLIANCE_POLICY["license_file"])
    copyright_file = _find_compliance_file(COMPLIANCE_POLICY["copyright_file"])
    third_party_file = _find_compliance_file(COMPLIANCE_POLICY["third_party_notice_file"])
    licensing_guide_file = _find_compliance_file(COMPLIANCE_POLICY["licensing_guide_file"])
    audit_md = _find_compliance_file(COMPLIANCE_POLICY["license_audit_artifacts"][0])
    audit_json = _find_compliance_file(COMPLIANCE_POLICY["license_audit_artifacts"][1])

    flags = []
    advisories = []

    if license_state["parse_error"]:
        flags.append(
            {
                "code": "LIC_UNKNOWN",
                "target": license_state["file"] or "commercial_license_file",
                "message": f"Unable to parse commercial license file: {license_state['parse_error']}",
            }
        )

    if license_state["mode"] == "commercial" and not license_state["attested"]:
        flags.append(
            {
                "code": "LIC_COMMERCIAL_UNATTESTED",
                "target": "commercial_license",
                "message": (
                    "Commercial mode selected but no attestation found. "
                    "Provide --commercial-license-id, --commercial-license-file, "
                    "--commercial-licensed, or equivalent FULLBLEED_COMMERCIAL_* env vars."
                ),
            }
        )

    if commercial_attested:
        advisories.append(
            "Commercial license attested: AGPL source/audit gate checks were skipped."
        )

    if not license_file:
        flags.append(
            {
                "code": "LIC_MISSING_NOTICE",
                "target": COMPLIANCE_POLICY["license_file"],
                "message": "Missing primary license file",
            }
        )
    else:
        try:
            license_text = license_file.read_text(encoding="utf-8")
            if COMPLIANCE_POLICY["license_required_header"] not in license_text:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["license_file"],
                        "message": (
                            "Primary license file does not match expected AGPL text "
                            f"(missing header: {COMPLIANCE_POLICY['license_required_header']})."
                        ),
                    }
                )
            forbidden_markers = [
                marker
                for marker in COMPLIANCE_POLICY["license_forbidden_markers"]
                if marker.lower() in license_text.lower()
            ]
            if forbidden_markers:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["license_file"],
                        "message": (
                            "Primary license file contains disallowed marker(s): "
                            + ", ".join(forbidden_markers)
                        ),
                    }
                )
        except Exception as exc:
            flags.append(
                {
                    "code": "LIC_UNKNOWN",
                    "target": COMPLIANCE_POLICY["license_file"],
                    "message": f"Unable to read primary license file: {exc}",
                }
            )

    if not copyright_file:
        flags.append(
            {
                "code": "LIC_MISSING_NOTICE",
                "target": COMPLIANCE_POLICY["copyright_file"],
                "message": "Missing copyright notice file",
            }
        )
    else:
        try:
            copyright_text = copyright_file.read_text(encoding="utf-8")
            required_spdx = COMPLIANCE_POLICY["copyright_required_spdx"]
            if required_spdx not in copyright_text:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["copyright_file"],
                        "message": "Copyright notice missing SPDX expression: " + required_spdx,
                    }
                )
            lowered_copyright = copyright_text.lower()
            missing_markers = [
                marker
                for marker in COMPLIANCE_POLICY["copyright_required_markers"]
                if marker.lower() not in lowered_copyright
            ]
            if missing_markers:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["copyright_file"],
                        "message": (
                            "Copyright notice missing required dual-license marker(s): "
                            + ", ".join(missing_markers)
                        ),
                    }
                )
        except Exception as exc:
            flags.append(
                {
                    "code": "LIC_UNKNOWN",
                    "target": COMPLIANCE_POLICY["copyright_file"],
                    "message": f"Unable to read copyright notice: {exc}",
                }
            )

    if not licensing_guide_file:
        flags.append(
            {
                "code": "LIC_MISSING_NOTICE",
                "target": COMPLIANCE_POLICY["licensing_guide_file"],
                "message": "Missing dual-license guide file",
            }
        )
    else:
        try:
            licensing_text = licensing_guide_file.read_text(encoding="utf-8")
            if LICENSE_SPDX_EXPRESSION not in licensing_text:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["licensing_guide_file"],
                        "message": "Licensing guide missing SPDX expression: " + LICENSE_SPDX_EXPRESSION,
                    }
                )
            if "dual-licensed" not in licensing_text.lower():
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["licensing_guide_file"],
                        "message": "Licensing guide should explicitly state dual-licensed terms.",
                    }
                )
            mojibake_markers = ["â€”", "â€™", "â€œ", "â€", "\ufffd"]
            bad_markers = [token for token in mojibake_markers if token in licensing_text]
            if bad_markers:
                flags.append(
                    {
                        "code": "LIC_POLICY_MISMATCH",
                        "target": COMPLIANCE_POLICY["licensing_guide_file"],
                        "message": (
                            "Licensing guide contains encoding artifacts (mojibake): "
                            + ", ".join(bad_markers)
                        ),
                    }
                )
        except Exception as exc:
            flags.append(
                {
                    "code": "LIC_UNKNOWN",
                    "target": COMPLIANCE_POLICY["licensing_guide_file"],
                    "message": f"Unable to read licensing guide: {exc}",
                }
            )

    if not commercial_attested and not third_party_file:
        flags.append(
            {
                "code": "LIC_MISSING_NOTICE",
                "target": COMPLIANCE_POLICY["third_party_notice_file"],
                "message": "Missing third-party notices file",
            }
        )

    audit_date = None
    audit_age_days = None
    audit_issue_count = None
    audit_skipped = commercial_attested
    if not commercial_attested and audit_json:
        try:
            audit_payload = json.loads(audit_json.read_text(encoding="utf-8"))
            raw_date = (audit_payload.get("summary") or {}).get("audit_date")
            if raw_date:
                audit_date = datetime.strptime(raw_date, "%Y-%m-%d").date().isoformat()
                audit_age_days = (now.date() - datetime.strptime(raw_date, "%Y-%m-%d").date()).days
            issues = audit_payload.get("issues") or []
            audit_issue_count = len(issues)
            if issues:
                flags.append(
                    {
                        "code": "LIC_DISALLOWED",
                        "target": "FONT_LICENSE_AUDIT.json",
                        "message": f"License audit reported {len(issues)} issue(s)",
                    }
                )
        except Exception as exc:
            flags.append(
                {
                    "code": "LIC_UNKNOWN",
                    "target": "FONT_LICENSE_AUDIT.json",
                    "message": f"Unable to parse license audit JSON: {exc}",
                }
            )
    elif not commercial_attested:
        flags.append(
            {
                "code": "LIC_AUDIT_STALE",
                "target": "FONT_LICENSE_AUDIT.json",
                "message": "Missing license audit JSON artifact",
            }
        )

    if (
        not commercial_attested
        and audit_age_days is not None
        and audit_age_days > max_age_days
    ):
        flags.append(
            {
                "code": "LIC_AUDIT_STALE",
                "target": "FONT_LICENSE_AUDIT.json",
                "message": f"License audit is {audit_age_days} days old (max {max_age_days})",
            }
        )

    if not commercial_attested and third_party_file:
        required_entries = [
            "bootstrap.min.css",
            "bootstrap-icons.svg",
            "NotoSans-Regular.ttf",
            "NotoSansMath-Regular.ttf",
            "NotoSansSymbols-Regular.ttf",
            "NotoSansSymbols2-Regular.ttf",
        ]
        try:
            notice_text = third_party_file.read_text(encoding="utf-8")
            missing_entries = [entry for entry in required_entries if entry not in notice_text]
            if missing_entries:
                flags.append(
                    {
                        "code": "LIC_ASSET_UNMAPPED",
                        "target": "THIRD_PARTY_LICENSES.md",
                        "message": "Bundled asset(s) missing from third-party notice",
                        "missing": missing_entries,
                    }
                )
        except Exception as exc:
            flags.append(
                {
                    "code": "LIC_UNKNOWN",
                    "target": "THIRD_PARTY_LICENSES.md",
                    "message": f"Unable to read third-party notices: {exc}",
                }
            )

    report = {
        "schema": "fullbleed.compliance.v1",
        "ok": len(flags) == 0,
        "policy": COMPLIANCE_POLICY,
        "license": {
            "spdx_expression": LICENSE_SPDX_EXPRESSION,
            "mode": license_state["mode"],
            "commercial": {
                "attested": commercial_attested,
                "license_id": license_state["license_id"],
                "company": license_state["company"],
                "tier": license_state["tier"],
                "source": license_state["source"],
                "file": license_state["file"],
            },
        },
        "runtime": {
            "cli_version": _get_version(),
            "python": sys.version.split()[0],
            "platform": sys.platform,
            "cwd": str(Path.cwd()),
            "checked_at_utc": now.isoformat(),
        },
        "files": {
            "license": str(license_file) if license_file else None,
            "copyright": str(copyright_file) if copyright_file else None,
            "third_party_notice": str(third_party_file) if third_party_file else None,
            "licensing_guide": str(licensing_guide_file) if licensing_guide_file else None,
            "license_audit_md": str(audit_md) if audit_md else None,
            "license_audit_json": str(audit_json) if audit_json else None,
        },
        "audit": {
            "date": audit_date,
            "age_days": audit_age_days,
            "max_age_days": max_age_days,
            "issues_count": audit_issue_count,
            "skipped": audit_skipped,
        },
        "advisories": advisories,
        "flags": flags,
    }

    if args.json:
        sys.stdout.write(_json_dumps(report) + "\n")
    else:
        sys.stdout.write(f"schema: {report['schema']}\n")
        sys.stdout.write(f"ok: {report['ok']}\n")
        sys.stdout.write(f"package_license: {COMPLIANCE_POLICY['package_license']}\n")
        sys.stdout.write(f"license_mode: {report['license']['mode']}\n")
        if report["license"]["commercial"]["attested"]:
            sys.stdout.write("commercial_license_attested: true\n")
        for key, value in report["files"].items():
            sys.stdout.write(f"{key}: {value}\n")
        if advisories:
            sys.stdout.write("advisories:\n")
            for advisory in advisories:
                sys.stdout.write(f"  - {advisory}\n")
        if flags:
            sys.stdout.write("flags:\n")
            for flag in flags:
                sys.stdout.write(f"  - {flag['code']}: {flag['message']}\n")
        else:
            sys.stdout.write("flags: []\n")

    if getattr(args, "strict", False) and flags:
        raise SystemExit(3)


def cmd_capabilities(args):
    """CLI handler for machine-readable capability inspection."""
    commands = [
        "render",
        "verify",
        "plan",
        "run",
        "compliance",
        "debug-perf",
        "debug-jit",
        "doctor",
        "capabilities",
        "assets",
        "cache",
        "init",
        "new",
    ]
    payload = {
        "schema": "fullbleed.capabilities.v1",
        "commands": commands,
        "agent_flags": [
            "--json",
            "--json-only",
            "--schema",
            "--emit-manifest",
            "--emit-image",
            "--image-dpi",
            "--no-prompts",
            "--allow-fallbacks",
            "--repro-record",
            "--repro-check",
        ],
        "engine": {
            "batch_render": hasattr(fullbleed.PdfEngine, "render_pdf_batch"),
            "batch_render_parallel": hasattr(fullbleed.PdfEngine, "render_pdf_batch_parallel"),
            "glyph_report": hasattr(fullbleed.PdfEngine, "render_pdf_with_glyph_report"),
            "page_data": hasattr(fullbleed.PdfEngine, "render_pdf_with_page_data"),
            "image_pages": hasattr(fullbleed.PdfEngine, "render_image_pages"),
        },
        "svg": {
            "document_input": {
                "html_file_accepts_svg": True,
                "html_str_accepts_svg_markup": True,
                "inline_svg_in_html": True,
            },
            "asset_bundle": {
                "kind": "svg",
                "auto_kind_from_extension": True,
            },
            "engine_flags": {
                "svg_form_xobjects": True,
                "svg_raster_fallback": True,
            },
        },
        "profiles": list(PROFILES.keys()),
        "fail_on": FAIL_ON_CHOICES,
        "fallback_policy_flags": ["--allow-fallbacks"],
        "budget_flags": ["--budget-max-pages", "--budget-max-bytes", "--budget-max-ms"],
        "compliance": COMPLIANCE_POLICY,
    }
    if args.json:
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
    else:
        for key, value in payload.items():
            sys.stdout.write(f"{key}: {value}\n")


def cmd_run(args):
    """Run a Python module's engine factory to render a PDF.
    
    Usage:
      fullbleed run module:engine_name --html input.html --out output.pdf
      fullbleed run module:engine_name --html-str "<p>hi</p>" --out output.pdf
    
    The module should export a function (engine_name) that returns a PdfEngine.
    Alternatively, the module can export a variable (engine_name) that is a PdfEngine.
    """
    import importlib
    import importlib.util
    
    entrypoint = args.entrypoint
    if ":" not in entrypoint:
        raise ValueError(
            f"Invalid entrypoint format: {entrypoint}. Expected 'module:engine_name' or 'path/to/file.py:engine_name'"
        )
    
    module_path, engine_name = entrypoint.rsplit(":", 1)

    license_warning_emitted = _maybe_emit_run_license_warning(args)
    
    # Check if it's a file path or a module name
    if module_path.endswith(".py") or os.path.isfile(module_path):
        # Load from file path
        spec = importlib.util.spec_from_file_location("_fb_runner_module", module_path)
        if spec is None or spec.loader is None:
            raise ValueError(f"Could not load module from: {module_path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules["_fb_runner_module"] = module
        spec.loader.exec_module(module)
    else:
        # Import as a module
        try:
            module = importlib.import_module(module_path)
        except ModuleNotFoundError as e:
            raise ValueError(f"Could not import module: {module_path}. Error: {e}")
    
    # Get the engine factory or engine instance
    if not hasattr(module, engine_name):
        available = [n for n in dir(module) if not n.startswith("_")]
        raise ValueError(
            f"Module '{module_path}' has no attribute '{engine_name}'. Available: {', '.join(available[:10])}"
        )
    
    engine_or_factory = getattr(module, engine_name)
    
    # If it's callable, call it to get the engine
    if callable(engine_or_factory):
        engine = engine_or_factory()
    else:
        engine = engine_or_factory
    
    # Collect HTML and CSS
    html = _collect_html(args)
    css = _collect_css(args)
    
    # Render
    out_path = args.out
    if out_path == "-":
        pdf_bytes = engine.render_pdf(html, css)
        sys.stdout.buffer.write(pdf_bytes)
        bytes_written = len(pdf_bytes)
    else:
        bytes_written = engine.render_pdf_to_file(html, css, out_path)
    bytes_written = _normalize_bytes_written(bytes_written)
    
    outputs = {"pdf": None if out_path == "-" else out_path}
    
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.run_result.v1",
            "ok": True,
            "entrypoint": entrypoint,
            "bytes_written": bytes_written,
            "license_warning_emitted": license_warning_emitted,
            "outputs": outputs,
        }
        sys.stdout.write(_json_dumps(result) + "\n")
    else:
        if out_path != "-":
            sys.stdout.write(f"[ok] {entrypoint} -> {out_path} ({bytes_written} bytes)\n")


def _add_bool_flag(p, name, default):
    """Register paired boolean flags (`--x` and `--no-x`) on a parser."""
    dest = name.replace("-", "_")
    p.add_argument(f"--{name}", dest=dest, action="store_true")
    p.add_argument(f"--no-{name}", dest=dest, action="store_false")
    p.set_defaults(**{dest: default})


def _add_common_flags(p):
    """Register common render pipeline flags shared by render/verify/plan."""
    p.add_argument(
        "--html",
        help="Path to HTML input. Use a .svg file to render a standalone SVG document.",
    )
    p.add_argument(
        "--html-str",
        help="Inline HTML or SVG markup string.",
    )
    p.add_argument("--css", action="append")
    p.add_argument("--css-str", action="append")
    p.add_argument("--page-size")
    p.add_argument("--page-width")
    p.add_argument("--page-height")
    p.add_argument("--margin")
    p.add_argument("--page-margins")
    _add_bool_flag(p, "reuse-xobjects", True)
    _add_bool_flag(p, "svg-form-xobjects", False)
    _add_bool_flag(p, "svg-raster-fallback", False)
    _add_bool_flag(p, "unicode-support", True)
    _add_bool_flag(p, "shape-text", True)
    _add_bool_flag(p, "unicode-metrics", True)
    p.add_argument("--pdf-version")
    p.add_argument("--pdf-profile")
    p.add_argument("--output-intent-icc")
    p.add_argument("--output-intent-identifier")
    p.add_argument("--output-intent-info")
    p.add_argument("--output-intent-components", type=int)
    p.add_argument("--color-space")
    p.add_argument("--document-lang")
    p.add_argument("--document-title")
    p.add_argument("--header-each")
    p.add_argument("--header-html-each")
    p.add_argument("--footer-each")
    p.add_argument("--footer-html-each")
    p.add_argument("--watermark-text")
    p.add_argument("--watermark-html")
    p.add_argument("--watermark-image")
    p.add_argument(
        "--watermark-layer",
        default="overlay",
        choices=["background", "overlay", "underlay"],
        help="Watermark placement layer (legacy 'underlay' alias maps to 'background')",
    )
    p.add_argument(
        "--watermark-semantics",
        default="artifact",
        choices=["visual", "artifact", "ocg"],
    )
    p.add_argument("--watermark-opacity", type=float, default=0.15)
    p.add_argument("--watermark-rotation", type=float, default=0.0)
    p.add_argument("--jit-mode")
    p.add_argument("--emit-jit")
    p.add_argument("--emit-perf")
    p.add_argument("--emit-glyph-report")
    p.add_argument("--emit-page-data")
    p.add_argument(
        "--emit-image",
        "--emit-images-dir",
        dest="emit_image",
        help="Write per-page PNG artifacts to this directory",
    )
    p.add_argument(
        "--image-dpi",
        "--emit-images-dpi",
        dest="image_dpi",
        type=int,
        default=150,
        help="DPI used for PNG image artifacts (default: 150)",
    )
    p.add_argument("--emit-manifest",
                   help="Write compiler input manifest derived from CLI flags")
    p.add_argument(
        "--asset",
        action="append",
        help="Repeatable asset path. .svg is inferred as asset kind 'svg'.",
    )
    p.add_argument(
        "--asset-kind",
        action="append",
        help="Asset kind override: css, font, svg, image.",
    )
    p.add_argument("--asset-name", action="append")
    p.add_argument("--asset-trusted", action="store_true")
    p.add_argument("--allow-remote-assets", action="store_true")
    p.add_argument("--verbose-assets", action="store_true")
    # Profile and fail-on flags
    p.add_argument("--profile", choices=["dev", "preflight", "prod"],
                   help="Apply profile presets (dev: fast/verbose, preflight: validation, prod: optimized)")
    p.add_argument(
        "--allow-fallbacks",
        action="store_true",
        help="Allow font/glyph fallback signals without failing fail-on checks",
    )
    p.add_argument("--fail-on", action="append", choices=FAIL_ON_CHOICES,
                   help="Exit non-zero on condition (repeatable): overflow, missing-glyphs, font-subst, budget")
    p.add_argument("--deterministic-hash",
                   help="Write SHA256 hash of output to this path for reproducibility checks")
    p.add_argument("--budget-max-pages", type=int,
                   help="Maximum allowed page count when used with --fail-on budget")
    p.add_argument("--budget-max-bytes", type=int,
                   help="Maximum allowed PDF size in bytes when used with --fail-on budget")
    p.add_argument("--budget-max-ms", type=float,
                   help="Maximum allowed render timing in milliseconds when used with --fail-on budget")
    p.add_argument("--repro-record",
                   help="Write reproducibility record JSON to this path")
    p.add_argument("--repro-check",
                   help="Check current render against an existing reproducibility record JSON")


def _build_parser():
    """Construct and return the top-level CLI parser."""
    parser = argparse.ArgumentParser(prog="fullbleed")
    parser.add_argument("--config")
    parser.add_argument("--log-level", choices=["error", "warn", "info", "debug"], default="info")
    parser.add_argument("--no-color", action="store_true")
    parser.add_argument("--version", action="version", version="fullbleed-cli " + _get_version())
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--json-only", action="store_true",
                        help="Emit JSON only (implies --json and --no-prompts)")
    parser.add_argument("--no-prompts", action="store_true",
                        help="Disable interactive prompts")
    parser.add_argument("--schema", action="store_true",
                        help="Print JSON schema for the requested command and exit")
    sub = parser.add_subparsers(dest="command", required=True)

    p_render = sub.add_parser("render", help="Render HTML/CSS to PDF (optionally emit PNG pages)")
    _add_common_flags(p_render)
    p_render.add_argument("--out", required=True)
    p_render.set_defaults(func=cmd_render)

    p_verify = sub.add_parser("verify", help="Validation render path with optional PDF/PNG artifacts")
    _add_common_flags(p_verify)
    p_verify.add_argument("--emit-pdf")
    p_verify.set_defaults(func=cmd_verify)

    p_plan = sub.add_parser("plan")
    _add_common_flags(p_plan)
    p_plan.set_defaults(func=cmd_plan)

    p_perf = sub.add_parser("debug-perf")
    p_perf.add_argument("--perf-log", required=True)
    p_perf.add_argument("--top", type=int, default=20)
    p_perf.add_argument("--json", action="store_true")
    p_perf.set_defaults(func=cmd_debug_perf)

    p_jit = sub.add_parser("debug-jit")
    p_jit.add_argument("--jit-log", required=True)
    p_jit.add_argument("--errors-only", action="store_true")
    p_jit.add_argument("--json", action="store_true")
    p_jit.set_defaults(func=cmd_debug_jit)

    p_doc = sub.add_parser("doctor")
    p_doc.add_argument("--strict", action="store_true")
    p_doc.add_argument("--json", action="store_true")
    p_doc.set_defaults(func=cmd_doctor)

    p_cap = sub.add_parser("capabilities", help="Machine-readable CLI and engine capability map")
    p_cap.add_argument("--json", action="store_true")
    p_cap.set_defaults(func=cmd_capabilities)

    p_compliance = sub.add_parser("compliance", help="License/compliance report for legal and procurement review")
    p_compliance.add_argument("--json", action="store_true")
    p_compliance.add_argument("--strict", action="store_true",
                              help="Exit non-zero when any compliance flag is present")
    p_compliance.add_argument("--max-audit-age-days", type=int, default=180,
                              help="Maximum allowed age for license audit artifacts")
    p_compliance.add_argument(
        "--license-mode",
        choices=["auto", "agpl", "commercial"],
        default="auto",
        help="Compliance basis. 'commercial' expects attestation via flags/env.",
    )
    p_compliance.add_argument(
        "--commercial-licensed",
        action="store_true",
        help="Attest that your organization holds a Fullbleed commercial license.",
    )
    p_compliance.add_argument(
        "--commercial-license-id",
        help="Commercial license/order identifier (or set FULLBLEED_COMMERCIAL_LICENSE_ID).",
    )
    p_compliance.add_argument(
        "--commercial-license-file",
        help="Path to commercial license attestation file (JSON or plain text id).",
    )
    p_compliance.add_argument(
        "--commercial-company",
        help="Company/legal entity for commercial attestation metadata.",
    )
    p_compliance.add_argument(
        "--commercial-tier",
        help="Commercial revenue tier metadata (optional).",
    )
    p_compliance.set_defaults(func=cmd_compliance)

    # ===== Asset management commands =====
    from . import assets as assets_module
    from . import cache as cache_module
    
    p_assets = sub.add_parser("assets", help="Manage asset packages")
    assets_sub = p_assets.add_subparsers(dest="assets_command", required=True)
    
    p_assets_list = assets_sub.add_parser("list", help="List installed packages")
    p_assets_list.add_argument("--available", "-a", action="store_true", help="Show available remote packages")
    p_assets_list.add_argument("--json", action="store_true")
    p_assets_list.set_defaults(func=assets_module.cmd_assets_list)
    
    p_assets_info = assets_sub.add_parser("info", help="Show package details")
    p_assets_info.add_argument("package", help="Package name (e.g., bootstrap, bootstrap-icons, noto-sans)")
    p_assets_info.add_argument("--json", action="store_true")
    p_assets_info.set_defaults(func=assets_module.cmd_assets_info)
    
    p_assets_install = assets_sub.add_parser("install", help="Install a package")
    p_assets_install.add_argument("package", help="Package reference (e.g., @bootstrap, @bootstrap-icons)")
    p_assets_install.add_argument("--vendor", help="Install to custom vendor directory")
    p_assets_install.add_argument("--global", "-g", dest="global_", action="store_true", 
                                   help="Install to global cache instead of ./vendor/")
    p_assets_install.add_argument("--json", action="store_true")
    p_assets_install.set_defaults(func=assets_module.cmd_assets_install)
    
    p_assets_verify = assets_sub.add_parser("verify", help="Verify package integrity")
    p_assets_verify.add_argument("package", help="Package name")
    p_assets_verify.add_argument(
        "--lock",
        nargs="?",
        const="assets.lock.json",
        help="Validate package against lock file (default: assets.lock.json)",
    )
    p_assets_verify.add_argument("--strict", action="store_true", help="Exit non-zero on lock mismatch")
    p_assets_verify.add_argument("--json", action="store_true")
    p_assets_verify.set_defaults(func=assets_module.cmd_assets_verify)
    
    p_assets_lock = assets_sub.add_parser("lock", help="Create/update assets.lock.json")
    p_assets_lock.add_argument("--add", action="append", help="Add package to lock file")
    p_assets_lock.add_argument("--output", default="assets.lock.json", help="Output path")
    p_assets_lock.add_argument("--json", action="store_true")
    p_assets_lock.set_defaults(func=assets_module.cmd_assets_lock)
    
    # ===== Cache management commands =====
    p_cache = sub.add_parser("cache", help="Manage asset cache")
    cache_sub = p_cache.add_subparsers(dest="cache_command", required=True)
    
    p_cache_dir = cache_sub.add_parser("dir", help="Print cache directory path")
    p_cache_dir.add_argument("--json", action="store_true")
    p_cache_dir.set_defaults(func=cache_module.cmd_cache_dir)
    
    p_cache_prune = cache_sub.add_parser("prune", help="Remove old cached packages")
    p_cache_prune.add_argument("--max-age-days", type=int, default=90, help="Max age in days")
    p_cache_prune.add_argument("--dry-run", action="store_true", help="Show what would be removed")
    p_cache_prune.add_argument("--json", action="store_true")
    p_cache_prune.set_defaults(func=cache_module.cmd_cache_prune)

    # ===== Project scaffolding commands =====
    from . import scaffold as scaffold_module
    
    p_init = sub.add_parser("init", help="Initialize a new fullbleed project")
    p_init.add_argument("path", nargs="?", default=".", help="Directory to initialize (default: current)")
    p_init.add_argument("--force", action="store_true", help="Overwrite existing files")
    p_init.add_argument("--json", action="store_true")
    p_init.set_defaults(func=scaffold_module.cmd_init)
    
    p_new = sub.add_parser("new", help="Create from a starter template")
    p_new.add_argument("template", help="Template name (invoice, statement)")
    p_new.add_argument("path", nargs="?", default=".", help="Target directory")
    p_new.add_argument("--force", action="store_true", help="Overwrite existing files")
    p_new.add_argument("--json", action="store_true")
    p_new.set_defaults(func=scaffold_module.cmd_new_template)

    # ===== Run command =====
    p_run = sub.add_parser("run", help="Run a Python module's engine factory")
    p_run.add_argument("entrypoint", help="Module:engine_name (e.g., report:engine or path/to/file.py:create_engine)")
    p_run.add_argument("--html", help="Path to HTML file or - for stdin")
    p_run.add_argument("--html-str", help="HTML string (alternative to --html)")
    p_run.add_argument("--css", action="append", help="Path to CSS file (repeatable)")
    p_run.add_argument("--css-str", action="append", help="CSS string (repeatable)")
    p_run.add_argument("--out", required=True, help="Output PDF path or - for stdout")
    p_run.add_argument("--no-license-warn", action="store_true",
                       help="Suppress one-time AGPL/commercial licensing reminder")
    p_run.add_argument("--json", action="store_true")
    p_run.set_defaults(func=cmd_run)

    return parser


def main(argv=None):
    """Execute CLI command dispatch and standardized error handling."""
    argv = list(sys.argv[1:] if argv is None else argv)
    force_json = "--json" in argv
    if "--schema" in argv:
        schema_name = _infer_schema_from_argv(argv)
        if schema_name:
            _emit_schema(schema_name)
            return 0
        err = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": "SCHEMA_NOT_FOUND",
            "message": "Unable to infer schema. Provide a command (e.g. render) or subcommand (e.g. assets list).",
        }
        sys.stdout.write(json.dumps(err, ensure_ascii=True) + "\n")
        return 3

    parser = _build_parser()
    args = parser.parse_args(argv)
    if force_json:
        args.json = True
    _apply_global_flags(args)
    try:
        if getattr(args, "emit_manifest", None):
            if args.emit_manifest == "-":
                raise ValueError("--emit-manifest cannot be '-' (stdout). Provide a file path.")
            manifest = _build_manifest(args)
            Path(args.emit_manifest).write_text(
                json.dumps(manifest, ensure_ascii=True, indent=2, sort_keys=True),
                encoding="utf-8",
            )
        args.func(args)
    except Exception as exc:
        if args.json:
            err = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "CLI_ERROR",
                "message": str(exc),
            }
            sys.stdout.write(json.dumps(err, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] {exc}\n")
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())




