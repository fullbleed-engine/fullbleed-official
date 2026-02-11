# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
import argparse
import hashlib
import json
import os
import sys
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
    if path_or_dash == "-":
        return sys.stdin.read()
    return Path(path_or_dash).read_text(encoding="utf-8")


def _read_json_or_path(value):
    if value is None:
        return None
    path = Path(value)
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    return json.loads(value)


def _detect_remote_refs(html, css):
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


def _collect_css(args):
    css_parts = []
    for css_path in args.css or []:
        css_parts.append(_read_text(css_path))
    for css_str in args.css_str or []:
        css_parts.append(css_str)
    return "\n\n".join(css_parts)


def _collect_html(args):
    if args.html_str is not None:
        return args.html_str
    if args.html is None:
        raise ValueError("--html or --html-str is required")
    return _read_text(args.html)


def _build_bundle(args):
    bundle = fullbleed.AssetBundle()
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


def _build_engine(args):
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
        watermark_layer=args.watermark_layer,
        watermark_semantics=args.watermark_semantics,
        watermark_opacity=args.watermark_opacity,
        watermark_rotation=args.watermark_rotation,
        jit_mode=args.jit_mode,
        debug=bool(emit_jit),
        debug_out=emit_jit,
        perf=bool(emit_perf),
        perf_out=emit_perf,
    )
    if args.asset:
        bundle = _build_bundle(args)
        engine.register_bundle(bundle)
    return engine


def _write_pdf_bytes(out_path, pdf_bytes):
    if out_path == "-":
        sys.stdout.buffer.write(pdf_bytes)
        return len(pdf_bytes)
    Path(out_path).write_bytes(pdf_bytes)
    return len(pdf_bytes)


def _render_with_artifacts(engine, html, css, out_path, args):
    page_data_path = args.emit_page_data
    glyph_path = args.emit_glyph_report
    
    # For fail-on checks, we need glyph report even if not explicitly requested
    fail_on = getattr(args, "fail_on", None) or []
    need_glyph_check = "missing-glyphs" in fail_on or "font-subst" in fail_on
    
    if page_data_path and glyph_path:
        sys.stderr.write(
            "[warn] both --emit-page-data and --emit-glyph-report set; rendering twice\n"
        )
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        Path(page_data_path).write_text(
            json.dumps(page_data, ensure_ascii=True, indent=2), encoding="utf-8"
        )
        _write_pdf_bytes(out_path, pdf_bytes)
        pdf_bytes, glyph = engine.render_pdf_with_glyph_report(html, css)
        Path(glyph_path).write_text(
            json.dumps(glyph, ensure_ascii=True, indent=2), encoding="utf-8"
        )
        return len(pdf_bytes), glyph, pdf_bytes

    if page_data_path:
        pdf_bytes, page_data = engine.render_pdf_with_page_data(html, css)
        Path(page_data_path).write_text(
            json.dumps(page_data, ensure_ascii=True, indent=2), encoding="utf-8"
        )
        # If we need glyph check, render again for glyph report
        if need_glyph_check:
            pdf_bytes2, glyph = engine.render_pdf_with_glyph_report(html, css)
            return _write_pdf_bytes(out_path, pdf_bytes), glyph, pdf_bytes
        return _write_pdf_bytes(out_path, pdf_bytes), None, pdf_bytes
    
    if glyph_path:
        pdf_bytes, glyph = engine.render_pdf_with_glyph_report(html, css)
        Path(glyph_path).write_text(
            json.dumps(glyph, ensure_ascii=True, indent=2), encoding="utf-8"
        )
        return _write_pdf_bytes(out_path, pdf_bytes), glyph, pdf_bytes

    if out_path == "-":
        pdf_bytes = engine.render_pdf(html, css)
        return _write_pdf_bytes(out_path, pdf_bytes), None, None
    
    # For fail-on checks, we need glyph report even if not explicitly requested
    fail_on = getattr(args, "fail_on", None) or []
    need_glyph_check = "missing-glyphs" in fail_on or "font-subst" in fail_on
    
    if need_glyph_check:
        pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report(html, css)
        bytes_written = _write_pdf_bytes(out_path, pdf_bytes)
        return bytes_written, glyph_report, pdf_bytes
    
    bytes_written = engine.render_pdf_to_file(html, css, out_path)
    return bytes_written, None, None


def _emit_result(ok, schema, out_path, bytes_written, outputs, args, error=None):
    if not args.json:
        if ok:
            msg = f"[ok] wrote {out_path} ({bytes_written} bytes)"
        else:
            msg = f"[error] {error.get('code')}: {error.get('message')}"
        sys.stdout.write(msg + "\n")
        return
    payload = {"schema": schema, "ok": ok, "outputs": outputs}
    if ok:
        payload["bytes_written"] = bytes_written
    else:
        payload.update(error or {})
    sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")


def cmd_render(args):
    html = _collect_html(args)
    css = _collect_css(args)
    if args.json and args.out == "-":
        raise ValueError("--json cannot be used with --out - (stdout PDF)")
    remote_refs = _detect_remote_refs(html, css)
    if remote_refs and not args.allow_remote_assets:
        sys.stderr.write(
            "[warn] remote asset refs detected; use --asset and --allow-remote-assets if needed\n"
        )
    engine = _build_engine(args)
    bytes_written, glyph_report, pdf_bytes = _render_with_artifacts(engine, html, css, args.out, args)
    
    # Fail-on validation
    fail_on = getattr(args, "fail_on", None) or []
    failures = []
    
    if "missing-glyphs" in fail_on and glyph_report:
        if glyph_report:  # glyph_report is a list of missing glyph entries
            failures.append({
                "code": "MISSING_GLYPHS",
                "message": f"Missing glyphs detected: {len(glyph_report)} unique codepoints",
                "count": len(glyph_report),
            })
    
    # Deterministic hash
    deterministic_hash = getattr(args, "deterministic_hash", None)
    output_hash = None
    if deterministic_hash and args.out != "-":
        # Read the output file to compute hash
        pdf_path = Path(args.out)
        if pdf_path.exists():
            file_bytes = pdf_path.read_bytes()
            output_hash = hashlib.sha256(file_bytes).hexdigest()
            Path(deterministic_hash).write_text(output_hash, encoding="utf-8")
    elif deterministic_hash and pdf_bytes:
        output_hash = hashlib.sha256(pdf_bytes).hexdigest()
        Path(deterministic_hash).write_text(output_hash, encoding="utf-8")
    
    outputs = {
        "pdf": None if args.out == "-" else args.out,
        "jit": args.emit_jit,
        "perf": args.emit_perf,
        "glyph_report": args.emit_glyph_report,
        "page_data": args.emit_page_data,
        "deterministic_hash": deterministic_hash,
        "sha256": output_hash,
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
    html = _collect_html(args)
    css = _collect_css(args)
    if args.emit_pdf and args.json and args.emit_pdf == "-":
        raise ValueError("--json cannot be used with --emit-pdf - (stdout PDF)")
    remote_refs = _detect_remote_refs(html, css)
    if remote_refs and not args.allow_remote_assets:
        sys.stderr.write(
            "[warn] remote asset refs detected; use --asset and --allow-remote-assets if needed\n"
        )
    if args.emit_jit and not args.emit_pdf and not args.jit_mode:
        args.jit_mode = "plan"
    engine = _build_engine(args)
    out_path = args.emit_pdf or "-"
    bytes_written = _render_with_artifacts(engine, html, css, out_path, args)
    outputs = {
        "pdf": None if out_path == "-" else out_path,
        "jit": args.emit_jit,
        "perf": args.emit_perf,
        "glyph_report": args.emit_glyph_report,
        "page_data": args.emit_page_data,
    }
    _emit_result(True, "fullbleed.verify_result.v1", out_path, bytes_written, outputs, args)


def _load_json_lines(path):
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
    report = {
        "python": sys.version.split()[0],
        "platform": sys.platform,
        "pdf_versions": ["1.7", "2.0"],
        "pdf_profiles": ["none", "pdfa2b", "pdfx4", "tagged"],
        "color_spaces": ["rgb", "cmyk"],
        "assets": {
            "bootstrap": str(fullbleed_assets.asset_path("bootstrap.min.css")),
            "noto_sans": str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")),
        },
    }
    if args.json:
        sys.stdout.write(json.dumps(report, ensure_ascii=True) + "\n")
    else:
        for k, v in report.items():
            sys.stdout.write(f"{k}: {v}\n")


def cmd_run(args):
    """Run a Python module's engine factory to render a PDF.
    
    Usage: fullbleed run module:engine_name --html input.html --out output.pdf
    
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
    
    outputs = {"pdf": None if out_path == "-" else out_path}
    
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.run_result.v1",
            "ok": True,
            "entrypoint": entrypoint,
            "bytes_written": bytes_written,
            "outputs": outputs,
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        if out_path != "-":
            sys.stdout.write(f"[ok] {entrypoint} -> {out_path} ({bytes_written} bytes)\n")


def _add_bool_flag(p, name, default):
    dest = name.replace("-", "_")
    p.add_argument(f"--{name}", dest=dest, action="store_true")
    p.add_argument(f"--no-{name}", dest=dest, action="store_false")
    p.set_defaults(**{dest: default})


def _add_common_flags(p):
    p.add_argument("--html")
    p.add_argument("--html-str")
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
    p.add_argument("--watermark-layer", default="overlay")
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
    p.add_argument("--asset", action="append")
    p.add_argument("--asset-kind", action="append")
    p.add_argument("--asset-name", action="append")
    p.add_argument("--asset-trusted", action="store_true")
    p.add_argument("--allow-remote-assets", action="store_true")
    p.add_argument("--verbose-assets", action="store_true")
    # Profile and fail-on flags
    p.add_argument("--profile", choices=["dev", "preflight", "prod"],
                   help="Apply profile presets (dev: fast/verbose, preflight: validation, prod: optimized)")
    p.add_argument("--fail-on", action="append", choices=FAIL_ON_CHOICES,
                   help="Exit non-zero on condition (repeatable): overflow, missing-glyphs, font-subst, budget")
    p.add_argument("--deterministic-hash",
                   help="Write SHA256 hash of output to this path for reproducibility checks")


def _build_parser():
    parser = argparse.ArgumentParser(prog="fullbleed")
    parser.add_argument("--json", action="store_true")
    sub = parser.add_subparsers(dest="command", required=True)

    p_render = sub.add_parser("render")
    _add_common_flags(p_render)
    p_render.add_argument("--out", required=True)
    p_render.set_defaults(func=cmd_render)

    p_verify = sub.add_parser("verify")
    _add_common_flags(p_verify)
    p_verify.add_argument("--emit-pdf")
    p_verify.set_defaults(func=cmd_verify)
    
    # ===== Compiler Workflow =====
    from . import builder as builder_module
    from . import watcher as watcher_module
    
    p_build = sub.add_parser("build", help="Build project from fullbleed.toml")
    p_build.add_argument("--config", help="Path to fullbleed.toml")
    # Allow overriding output path
    p_build.add_argument("--out", help="Output PDF path (overrides config)")
    p_build.add_argument("--json", action="store_true")
    p_build.set_defaults(func=builder_module.cmd_build)
    
    p_watch = sub.add_parser("watch", help="Watch project for changes and rebuild")
    p_watch.add_argument("--config", help="Path to fullbleed.toml")
    p_watch.set_defaults(func=watcher_module.cmd_watch)

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
    p_doc.add_argument("--json", action="store_true")
    p_doc.set_defaults(func=cmd_doctor)

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
    p_assets_info.add_argument("package", help="Package name (e.g., bootstrap, noto-sans)")
    p_assets_info.add_argument("--json", action="store_true")
    p_assets_info.set_defaults(func=assets_module.cmd_assets_info)
    
    p_assets_install = assets_sub.add_parser("install", help="Install a package")
    p_assets_install.add_argument("package", help="Package reference (e.g., @bootstrap)")
    p_assets_install.add_argument("--vendor", help="Install to custom vendor directory")
    p_assets_install.add_argument("--global", "-g", dest="global_", action="store_true", 
                                   help="Install to global cache instead of ./vendor/")
    p_assets_install.add_argument("--json", action="store_true")
    p_assets_install.set_defaults(func=assets_module.cmd_assets_install)
    
    p_assets_verify = assets_sub.add_parser("verify", help="Verify package integrity")
    p_assets_verify.add_argument("package", help="Package name")
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
    p_run.add_argument("--html", required=True, help="Path to HTML file or - for stdin")
    p_run.add_argument("--html-str", help="HTML string (alternative to --html)")
    p_run.add_argument("--css", action="append", help="Path to CSS file (repeatable)")
    p_run.add_argument("--css-str", action="append", help="CSS string (repeatable)")
    p_run.add_argument("--out", required=True, help="Output PDF path or - for stdout")
    p_run.add_argument("--json", action="store_true")
    p_run.set_defaults(func=cmd_run)

    return parser


def main(argv=None):
    parser = _build_parser()
    args = parser.parse_args(argv)
    try:
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
