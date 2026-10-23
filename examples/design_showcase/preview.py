#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Edit a showcase locally and see finalized PDF pages refresh after each save.

This optional example uses Fullbleed and Python's standard library. It executes
your local Python source; it is a development tool, not a hosted rendering API.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent
IGNORED = {".git", ".venv", "venv", "__pycache__", "node_modules", "target", "output", ".pytest_cache"}
TITLES = {"invoice": "Northstar Studio — Invoice NS-1042", "report": "Common Ground — Community Review 2026"}


def render_worker(args):
    """Every rebuild has fresh module state, fonts, data and native engine state."""
    from importlib.metadata import version
    import fullbleed
    import render as original

    out = args._render
    if args.authoring == "components":
        from fullbleed.ui.experimental import Renderer
        if args.document == "invoice":
            from invoice_components import northstar_invoice
            document = northstar_invoice(original.DATA["invoice"])
        else:
            from report_components import common_ground_report
            document = common_ground_report(original.DATA["report"])
        renderer = Renderer(font_files=[str(path) for path in original.FONTS])
        engine = renderer.engine_for(document)
        prepared = document.prepare(renderer)
        exports = {
            "document.styled.html": document.to_html(include_styles=True),
            "document.unstyled.html": document.to_html(include_styles=False),
        }
    else:
        html, css = original.sources(args.document)
        engine = fullbleed.PdfEngine(font_files=[str(path) for path in original.FONTS],
                                    document_title=TITLES[args.document], document_lang="en-US")
        prepared = engine.compile_pdf(html, css)
        exports = {"source.html": html, "styles.css": css}
    pdf = prepared.render_pdf()
    path = out / "document.pdf"
    path.write_bytes(pdf)
    inspection = fullbleed.inspect_pdf(str(path))
    if not 1 <= inspection["page_count"] <= 100:
        raise ValueError("This local preview supports documents of 1–100 pages")
    # Preview the saved PDF, including finalized transparency and paint order.
    pages = engine.render_finalized_pdf_image_pages(str(path), dpi=args.dpi)
    if len(pages) != inspection["page_count"]:
        raise RuntimeError("Saved-PDF preview page count differs from PDF inspection")
    files = ["document.pdf", *exports]
    for index, data in enumerate(pages, 1):
        name = f"page-{index}.png"
        (out / name).write_bytes(data)
        files.append(name)
    for name, text in exports.items():
        (out / name).write_text(text, encoding="utf-8", newline="\n")
    record = dict(title=TITLES[args.document], engine_version=version("fullbleed"),
                  page_count=len(pages), pdf_sha256=sha256(pdf).hexdigest(),
                  pdf_bytes=len(pdf), files=files, dpi=args.dpi)
    (out / "result.json").write_text(json.dumps(record), encoding="utf-8")


def snapshot(roots):
    """Notice edits, atomic saves and missing files without following directory links."""
    result = {}

    def remember(path):
        try:
            stat = path.stat()
            result[str(path)] = (stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns, stat.st_ino)
        except OSError as error:
            result[str(path)] = ("unavailable", error.errno)

    for root in roots:
        if not root.is_dir():
            remember(root)
            continue
        def failed(error):
            result[str(error.filename)] = ("unavailable", error.errno)
        for directory, folders, files in os.walk(root, onerror=failed, followlinks=False):
            directory = Path(directory)
            folders[:] = [name for name in folders if name not in IGNORED
                          and not (directory / name).is_symlink()]
            for name in files:
                remember(directory / name)
    return result


@dataclass(frozen=True)
class Revision:
    number: int
    metadata: dict
    files: dict[str, bytes]


class Preview:
    def __init__(self, args):
        self.args = args
        self.roots = tuple(dict.fromkeys([ROOT, *(path.resolve() for path in args.watch_path)]))
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.phase = "building"
        self.error = ""
        self.revision = 0
        self.attempt = 0
        self.history: list[Revision] = []

    def status(self):
        with self.lock:
            latest = self.history[-1] if self.history else None
            return dict(phase=self.phase, error=self.error, attempt=self.attempt,
                        document=self.args.document, authoring=self.args.authoring,
                        revision=latest.number if latest else None,
                        result=latest.metadata if latest else None)

    def rebuild(self):
        with self.lock:
            self.phase, self.error = "building", ""
            self.attempt += 1
        with tempfile.TemporaryDirectory(prefix="fullbleed-preview-") as directory:
            out = Path(directory)
            environment = {**os.environ, "PYTHONIOENCODING": "utf-8", "PYTHONDONTWRITEBYTECODE": "1",
                           "PYTHONPYCACHEPREFIX": str(out / "bytecode")}
            # A new cache prefix also prevents reading an old same-second .pyc.
            command = [sys.executable, str(ROOT / "preview.py"), "--document", self.args.document,
                       "--authoring", self.args.authoring, "--dpi", str(self.args.dpi), "--_render", str(out)]
            with (out / "worker.log").open("wb") as log:
                process = subprocess.Popen(command, cwd=ROOT, env=environment,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                           creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
                                           start_new_session=os.name != "nt")
                deadline = time.monotonic() + self.args.timeout
                timed_out = False
                try:
                    while process.poll() is None and not self.stop.wait(0.05):
                        if time.monotonic() >= deadline:
                            timed_out = True
                            break
                finally:
                    if process.poll() is None:
                        # A venv launcher or authored component can spawn a
                        # descendant that inherits worker.log. Stop the whole
                        # worker tree before removing its temporary directory.
                        if os.name == "nt":
                            stopped = subprocess.run(
                                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                capture_output=True,
                            )
                            if stopped.returncode and process.poll() is None:
                                stopped.check_returncode()
                        else:
                            try:
                                os.killpg(process.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass  # The worker exited after poll().
                    process.wait()
            if self.stop.is_set():
                return None
            if timed_out:
                raise RuntimeError(f"Rendering exceeded {self.args.timeout:g} seconds. Save an edit to retry, "
                                   "or restart with a larger --timeout.")
            if process.returncode:
                with (out / "worker.log").open("rb") as log:
                    log.seek(max(0, log.seek(0, 2) - 12000))
                    message = log.read().decode("utf-8", errors="replace").strip()
                raise RuntimeError(message or f"Renderer exited with code {process.returncode}")
            metadata = json.loads((out / "result.json").read_text(encoding="utf-8"))
            names = metadata["files"]
            if any(Path(name).name != name or name in {".", ".."} for name in names):
                raise ValueError("Renderer returned an invalid artifact name")
            files = {name: (out / name).read_bytes() for name in names}
            return metadata, files

    def run(self):
        previous, due = snapshot(self.roots), time.monotonic()
        while not self.stop.is_set():
            current = snapshot(self.roots)
            if current != previous:
                previous, due = current, time.monotonic() + 0.25
            if due is not None and time.monotonic() >= due:
                before = previous
                try:
                    result = self.rebuild()
                    failure = ""
                except Exception as error:
                    result, failure = None, str(error)
                if self.stop.is_set():
                    break
                after = snapshot(self.roots)
                if after != before:
                    # Never publish a build made from a mixture of saved inputs.
                    previous, due = after, time.monotonic() + 0.25
                    continue
                with self.lock:
                    if result is not None:
                        self.revision += 1
                        self.history.append(Revision(self.revision, *result))
                        self.history = self.history[-2:]
                        self.phase, self.error = "ready", ""
                    else:
                        self.phase, self.error = "error", failure
                    event = dict(event="preview_result", phase=self.phase, revision=self.revision,
                                 attempt=self.attempt, error=failure.splitlines()[-1] if failure else "")
                print(json.dumps(event), flush=True)
                due = None
            self.stop.wait(0.1)


def handler(preview, token):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, code, body=b"", content_type="text/plain; charset=utf-8", *, filename=None):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Content-Security-Policy", "default-src 'none'; script-src 'self'; "
                             "style-src 'unsafe-inline'; img-src 'self'; connect-src 'self'; "
                             "base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
            if filename:
                self.send_header("Content-Disposition", f'attachment; filename="{filename}"')
            self.end_headers()
            if self.command != "HEAD":
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        def do_GET(self):
            origin = f"http://127.0.0.1:{self.server.server_port}"
            if self.headers.get("Host") != origin.removeprefix("http://") or (
                self.headers.get("Origin") not in {None, origin}
            ) or self.headers.get("Sec-Fetch-Site") == "cross-site":
                return self.reply(403, b"Local preview only")
            path = urlsplit(self.path).path
            prefix = f"/{token}/"
            if not path.startswith(prefix):
                return self.reply(404)
            relative = path[len(prefix):]
            if relative in {"", "preview.js"}:
                name = "preview.html" if not relative else relative
                kind = "text/html; charset=utf-8" if not relative else "text/javascript; charset=utf-8"
                return self.reply(200, (ROOT / name).read_bytes(), kind)
            if relative == "status":
                return self.reply(200, json.dumps(preview.status()).encode(), "application/json")
            parts = relative.split("/")
            if len(parts) == 3 and parts[0] == "revisions" and parts[1].isdigit():
                with preview.lock:
                    revision = next((item for item in preview.history if item.number == int(parts[1])), None)
                if revision and parts[2] in revision.files:
                    name, body = parts[2], revision.files[parts[2]]
                    mime = {".pdf": "application/pdf", ".png": "image/png", ".html": "text/html; charset=utf-8",
                            ".css": "text/css; charset=utf-8"}[Path(name).suffix]
                    return self.reply(200, body, mime, filename=None if name.endswith(".png") else name)
            self.reply(404)

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--document", choices=("invoice", "report"), default="invoice")
    parser.add_argument("--authoring", choices=("components", "html"), default="components")
    parser.add_argument("--port", type=int, default=0, help="Loopback port; default chooses a free port")
    parser.add_argument("--dpi", type=int, default=120)
    parser.add_argument("--timeout", type=float, default=60, help="Maximum seconds per rebuild")
    parser.add_argument("--watch-path", type=Path, action="append", default=[], help="Additional file or directory to watch")
    parser.add_argument("--_render", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not 0 <= args.port <= 65535 or not 36 <= args.dpi <= 300 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("Use a port between 0 and 65535, DPI between 36 and 300, and a finite positive timeout")
    if args._render is not None:
        render_worker(args)
        return 0
    preview = Preview(args)
    token = secrets.token_urlsafe(24)
    with ThreadingHTTPServer(("127.0.0.1", args.port), handler(preview, token)) as server:
        server.daemon_threads = True
        worker = threading.Thread(target=preview.run, name="showcase-render")
        print(json.dumps(dict(event="preview_started", url=f"http://127.0.0.1:{server.server_port}/{token}/",
                              document=args.document, authoring=args.authoring)), flush=True)
        print("Open the URL above. Save Python, CSS, HTML or data files to rebuild. Ctrl-C stops the preview.",
              file=sys.stderr, flush=True)
        worker.start()
        try:
            server.serve_forever(poll_interval=0.1)
        except KeyboardInterrupt:
            pass
        finally:
            preview.stop.set()
            worker.join()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
