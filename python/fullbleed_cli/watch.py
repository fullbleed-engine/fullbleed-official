# SPDX-License-Identifier: MIT
"""Dependency-free file polling for the render command."""

from __future__ import annotations

from copy import deepcopy
import json
import math
import os
from pathlib import Path
import sys
import time


IGNORED_DIRECTORIES = frozenset({
    ".git", ".hg", ".svn", ".venv", "venv", "node_modules", "__pycache__", "target",
})
OUTPUT_FILE_OPTIONS = (
    "out", "emit_jit", "emit_perf", "emit_glyph_report", "emit_page_data",
    "emit_compose_plan", "emit_manifest", "deterministic_hash", "repro_record",
)
JSON_INPUT_OPTIONS = ("page_margins", "pdf_vt_job", "template_binding", "templates")


def _path(value, *, json_value=False):
    if not value or str(value).lower().startswith(("https://", "http://", "data:")):
        return None
    if json_value:
        try:
            json.loads(value)
        except (ValueError, TypeError):
            pass
        else:
            # Inline JSON is not a filesystem dependency.
            return None
    return Path(os.path.abspath(value))


def _fingerprint(path):
    try:
        stat = path.stat()
        return (stat.st_mode, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns,
                stat.st_ino, stat.st_dev)
    except OSError as exc:
        # Missing files remain watched, including an editor's rename/save gap.
        return ("unavailable", exc.errno)


class WatchInputs:
    def __init__(self, args, asset_paths):
        if args.out == "-":
            raise ValueError("--watch requires a file for --out; PDF stdout is not supported")
        if getattr(args, "emit_manifest", None) == "-":
            raise ValueError("--emit-manifest cannot be '-' (stdout). Provide a file path.")
        raw_inputs = list(getattr(args, "css", None) or [])
        if getattr(args, "html_str", None) is None:
            if getattr(args, "html", None) is None:
                raise ValueError("--html or --html-str is required")
            raw_inputs.append(args.html)
        if "-" in raw_inputs:
            raise ValueError("--watch does not support stdin HTML or CSS; use input files")
        raw_inputs.extend(asset_paths)
        raw_inputs.extend(getattr(args, "watch_path", None) or [])
        raw_inputs.extend(getattr(args, option, None) for option in (
            "output_intent_icc", "watermark_image", "repro_check",
        ))
        inputs = [_path(value) for value in raw_inputs]
        inputs.extend(_path(getattr(args, option, None), json_value=True)
                      for option in JSON_INPUT_OPTIONS)
        self.roots = tuple(dict.fromkeys(path for path in inputs if path is not None))
        if not self.roots:
            raise ValueError("--watch needs a local input file or at least one --watch-path")

        self.output_files = {
            path.resolve() for option in OUTPUT_FILE_OPTIONS
            if (path := _path(getattr(args, option, None))) is not None
        }
        if getattr(args, "profile", None) == "preflight":
            if not getattr(args, "emit_jit", None):
                self.output_files.add(Path("fullbleed_preflight.jit").resolve())
            if not getattr(args, "emit_perf", None):
                self.output_files.add(Path("fullbleed_preflight.perf").resolve())
        image_dir = _path(getattr(args, "emit_image", None))
        self.image_dir = image_dir.resolve() if image_dir else None
        for path in self.roots:
            if self.excluded(path):
                raise ValueError(f"watch input overlaps a render output: {path}")

    def excluded(self, path):
        resolved = path.resolve()
        return resolved in self.output_files or (
            self.image_dir is not None and resolved.is_relative_to(self.image_dir)
        )

    def snapshot(self):
        result = {}
        for path in self.roots:
            if not path.is_dir():
                result[str(path)] = _fingerprint(path)
                continue
            visited = set()

            def on_error(exc):
                result[str(exc.filename)] = ("unavailable", exc.errno)

            for directory, folders, files in os.walk(path, onerror=on_error, followlinks=False):
                directory = Path(directory)
                resolved = directory.resolve()
                if resolved in visited:
                    folders[:] = []
                    continue
                visited.add(resolved)
                folders[:] = [name for name in folders
                              if name not in IGNORED_DIRECTORIES
                              and not self.excluded(directory / name)
                              and not (directory / name).is_symlink()]
                for name in files:
                    candidate = directory / name
                    if not self.excluded(candidate):
                        result[str(candidate)] = _fingerprint(candidate)
        return result


def _duration(value, option, *, allow_zero=False):
    if not math.isfinite(value) or value < 0 or (value == 0 and not allow_zero):
        qualifier = "non-negative" if allow_zero else "positive"
        raise ValueError(f"{option} must be a finite {qualifier} number of seconds")
    return value


def run(args, render_once, emit_error, *, asset_paths=()):
    """Render initially, then once after each stable input change until Ctrl-C."""
    interval = _duration(args.watch_interval, "--watch-interval")
    debounce = _duration(args.watch_debounce, "--watch-debounce", allow_zero=True)
    inputs = WatchInputs(args, asset_paths)
    previous_images = {}

    def status(message):
        if not getattr(args, "json_only", False):
            sys.stderr.write(f"[watch] {message}\n")
            sys.stderr.flush()

    def render():
        nonlocal previous_images
        current = deepcopy(args)
        current.watch = False
        current.watch_path = None
        try:
            outputs = render_once(current) or {}
        except SystemExit as exc:
            if exc.code != 1:
                raise
            # The render handler already emitted its quality-gate failure.
        except Exception as exc:
            emit_error(current, exc)
        else:
            images = {Path(path): _fingerprint(Path(path))
                      for path in outputs.get("image_paths") or []}
            for path, signature in previous_images.items():
                if path not in images and _fingerprint(path) == signature:
                    try:
                        path.unlink(missing_ok=True)
                    except OSError as exc:
                        status(f"could not remove old preview {path}: {exc}")
            previous_images = images
        finally:
            sys.stdout.flush()
            sys.stderr.flush()

    try:
        status(f"watching {len(inputs.roots)} input path(s); press Ctrl-C to stop")
        observed = inputs.snapshot()
        render()
        changed_at = None
        while True:
            time.sleep(interval)
            latest = inputs.snapshot()
            now = time.monotonic()
            if latest != observed:
                observed = latest
                changed_at = now
            if changed_at is not None and now - changed_at >= debounce:
                render()
                changed_at = None
                # Do not resnapshot here: changes made during rendering must
                # trigger another pass, instead of being silently swallowed.
    except KeyboardInterrupt:
        status("stopped")
        raise SystemExit(130) from None
