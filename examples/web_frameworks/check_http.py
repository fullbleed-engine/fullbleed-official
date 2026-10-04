# SPDX-License-Identifier: MIT
"""Exercise the documented local server commands using actual HTTP responses."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, build_opener, ProxyHandler

ROOT = Path(__file__).resolve().parent
OPEN = build_opener(ProxyHandler({})).open


def fetch(url: str, method: str = "GET"):
    try:
        response = OPEN(Request(url, method=method), timeout=20)
    except HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


@contextmanager
def server(name: str, out: Path):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    commands = {
        "fastapi": ["-m", "uvicorn", "fastapi_app:app", "--host", "127.0.0.1", "--port", str(port)],
        "flask": ["-m", "flask", "--app", "flask_app", "run", "--host", "127.0.0.1", "--port", str(port)],
        "django": ["django_app.py", "runserver", f"127.0.0.1:{port}", "--noreload"],
    }
    env = {key: os.environ[key] for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "HOME", "LANG"] if key in os.environ}
    env.update(PYTHONNOUSERSITE="1", PYTHONUNBUFFERED="1")
    origin = f"http://127.0.0.1:{port}"
    with (out / f"{name}-server.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen([sys.executable, *commands[name]], cwd=ROOT,
                                   env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 20
            while True:
                if process.poll() is not None:
                    raise AssertionError(f"{name} exited during startup; inspect its server log.")
                try:
                    assert fetch(origin)[0] == 200
                    break
                except URLError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError(f"{name} did not start in 20 seconds.")
                    time.sleep(0.1)
            yield origin
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)


def check_servers(out: Path, expected_pdf: bytes) -> list[dict]:
    records = []
    for name in ["fastapi", "flask", "django"]:
        with server(name, out) as origin:
            code, headers, page = fetch(origin)
            assert code == 200 and headers.get_content_type() == "text/html"
            # The app reads HTML as text, normalizing CRLF in Windows checkouts.
            expected_html = (ROOT / "static/index.html").read_text(encoding="utf-8")
            assert page.decode("utf-8") == expected_html
            code, headers, preview = fetch(origin + "/preview.png")
            assert code == 200 and headers.get_content_type() == "image/png"
            assert preview == (ROOT / "static/invoice.png").read_bytes()
            url = origin + "/invoices/INV-1042.pdf"
            with ThreadPoolExecutor(max_workers=2) as pool:
                responses = list(pool.map(fetch, [url, url]))
            for code, headers, pdf in responses:
                assert code == 200 and pdf == expected_pdf
                assert headers.get_content_type() == "application/pdf"
                assert headers["Content-Disposition"] == 'attachment; filename="invoice.pdf"'
                assert headers["Cache-Control"] == "private, no-store"
            assert fetch(origin + "/invoices/DOES-NOT-EXIST.pdf")[0] == 404
            assert fetch(url, "POST")[0] == 405
            (out / f"{name}-http.pdf").write_bytes(responses[0][2])
            records.append({"framework": name, "ok": True, "pdf_status": 200,
                            "missing_status": 404, "post_status": 405,
                            "parallel_responses_identical": True,
                            "sha256": hashlib.sha256(expected_pdf).hexdigest(),
                            "home_and_preview_match": True})
        records[-1]["server_stopped"] = True
    return records
