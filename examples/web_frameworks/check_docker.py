#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build the invoice image and verify real HTTP and offline rendering in Docker.

Requires a local Linux-container Docker daemon and pypdf on the host. Removes
only the uniquely named containers and image tag it creates; retains evidence.
"""
from __future__ import annotations

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import subprocess
import time
from urllib.error import URLError
from uuid import uuid4

from pypdf import PdfReader

from check_http import fetch

ROOT = Path(__file__).resolve().parent
LIMITS = ["--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
          "--memory", "512m", "--cpus", "2", "--pids-limit", "128"]
RUNTIME = r'''
import importlib.metadata as metadata
import json, os, platform, shutil, sys
from pathlib import Path
tools = ["cc", "gcc", "clang", "cargo", "rustc", "chromium", "google-chrome", "wkhtmltopdf", "gs", "pdftoppm"]
assert os.getuid() == 10001 and os.getgid() == 10001
assert Path("/app/invoice.py").stat().st_uid == 0
try:
    Path("/app/write-probe").write_text("must fail")
except OSError:
    pass
else:
    raise AssertionError("Application directory is writable")
assert not any(shutil.which(tool) for tool in tools)
system_fonts = [str(p) for root in ["/usr/share/fonts", "/usr/local/share/fonts"]
                for p in Path(root).rglob("*") if p.suffix.lower() in [".ttf", ".otf", ".ttc"]]
assert not system_fonts, system_fonts
report = json.loads(Path("/app/pip-install.json").read_text())
assert all(item["download_info"]["url"].endswith(".whl") for item in report["install"])
assert metadata.version("fullbleed") == "2.5.11"
print(json.dumps({"uid": os.getuid(), "gid": os.getgid(), "python": sys.version,
    "machine": platform.machine(), "os": platform.freedesktop_os_release(),
    "packages": sorted([{"name": d.metadata["Name"], "version": d.version} for d in metadata.distributions()], key=lambda d: d["name"]),
    "absent_tools": tools, "system_fonts": system_fonts,
    "application_root_owned_and_not_writable": True, "pip_install": report}))
'''
OFFLINE = r'''
import base64, hashlib, json, tempfile
from pathlib import Path
import fullbleed
from invoice import load_invoice, render_invoice
pdf = render_invoice(load_invoice("INV-1042"))
with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "invoice.pdf"
    path.write_bytes(pdf)
    report = dict(fullbleed.inspect_pdf(str(path)))
    assert report["page_count"] == 1 and report["profile"]["embedded_font_count"] >= 4
    assert not report["warnings"] and not report["composition"]["issues"]
    pages = fullbleed.PdfEngine().render_finalized_pdf_image_pages(str(path), 110)
    assert len(pages) == 1
    print(json.dumps({"pdf_sha256": hashlib.sha256(pdf).hexdigest(), "inspection": report,
        "preview_base64": base64.b64encode(pages[0]).decode("ascii")}))
'''


def docker(*args: str, input: str | None = None, timeout: int = 60) -> str:
    result = subprocess.run(["docker", *args], input=input, text=True,
                            encoding="utf-8", capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"docker {args[0]} failed: {result.stderr.strip()}")
    return result.stdout


def pdf_check(data: bytes) -> dict:
    reader = PdfReader(io.BytesIO(data))
    assert len(reader.pages) == 1
    text = "".join(reader.pages[0].extract_text().split())
    for marker in ["INV-1042", "Maple & Finch", "Design workshop", "USD 1,870.00"]:
        assert "".join(marker.split()) in text, marker
    assert text.count("Designworkshop") == 1
    fonts = reader.pages[0]["/Resources"]["/Font"].get_object()
    embedded = 0
    for reference in fonts.values():
        font = reference.get_object()
        if "/DescendantFonts" in font:
            font = font["/DescendantFonts"][0].get_object()
        descriptor = font.get("/FontDescriptor")
        if descriptor and any(key in descriptor.get_object() for key in ["/FontFile", "/FontFile2", "/FontFile3"]):
            embedded += 1
    assert embedded >= 4, embedded
    return {"page_count": 1, "embedded_fonts": embedded, "expected_text_and_total": True,
            "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--base-image", help="Optional pinned official Python image; defaults to the Dockerfile's slim image.")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    name = "fullbleed-invoice-check-" + uuid4().hex
    offline_name = name + "-offline"
    image = name + ":test"
    receipt = {"ok": False, "checked_at": datetime.now(timezone.utc).isoformat(),
               "base_image_override": args.base_image, "limits": LIMITS}
    built = False
    started = False
    try:
        receipt["docker_version"] = json.loads(docker("version", "--format", "{{json .}}"))
        build = ["docker", "build", "--pull", "--progress", "plain", "--tag", image]
        if args.base_image:
            build += ["--build-arg", "PYTHON_IMAGE=" + args.base_image]
        build += [str(ROOT)]
        with (out / "build.log").open("w", encoding="utf-8") as log:
            result = subprocess.run(build, stdout=log, stderr=subprocess.STDOUT, timeout=600)
        if result.returncode:
            raise RuntimeError(f"Image build failed; inspect {out / 'build.log'}")
        built = True
        details = json.loads(docker("image", "inspect", image))[0]
        receipt["image"] = {key: details[key] for key in ["Id", "Architecture", "Os", "Size"]}
        assert details["Config"]["User"] == "10001:10001"
        # Ephemeral loopback port: safe alongside other local applications.
        docker("run", "--detach", "--name", name, *LIMITS, "--publish", "127.0.0.1::8000", image)
        started = True
        details = json.loads(docker("container", "inspect", name))[0]
        bindings = details["NetworkSettings"]["Ports"]["8000/tcp"]
        assert len(bindings) == 1 and bindings[0]["HostIp"] == "127.0.0.1"
        config = details["HostConfig"]
        assert config["ReadonlyRootfs"] and config["CapDrop"] == ["ALL"]
        assert config["Memory"] == 512 * 1024 * 1024 and config["NanoCpus"] == 2_000_000_000
        assert "no-new-privileges:true" in config["SecurityOpt"]
        origin = "http://127.0.0.1:" + bindings[0]["HostPort"]
        deadline = time.monotonic() + 40
        while True:
            try:
                status, headers, body = fetch(origin)
                assert status == 200 and headers.get_content_type() == "text/html"
                assert body.decode() == (ROOT / "static/index.html").read_text(encoding="utf-8")
                break
            except (URLError, ConnectionError):
                if time.monotonic() > deadline:
                    raise TimeoutError("Invoice server did not become ready")
                time.sleep(0.2)
        url = origin + "/invoices/INV-1042.pdf"
        with ThreadPoolExecutor(max_workers=4) as pool:
            responses = list(pool.map(fetch, [url] * 4))
        for status, headers, pdf in responses:
            assert status == 200 and headers.get_content_type() == "application/pdf"
            assert headers["Content-Disposition"] == 'attachment; filename="invoice.pdf"'
            assert headers["Cache-Control"] == "private, no-store"
            assert pdf == responses[0][2]
        (out / "invoice.pdf").write_bytes(responses[0][2])
        receipt["pdf"] = pdf_check(responses[0][2])
        assert fetch(origin + "/invoices/DOES-NOT-EXIST.pdf")[0] == 404
        assert fetch(url, "POST")[0] == 405
        status, headers, preview = fetch(origin + "/preview.png")
        assert status == 200 and headers.get_content_type() == "image/png"
        assert preview == (ROOT / "static/invoice.png").read_bytes()
        receipt["runtime"] = json.loads(docker("exec", "-i", name, "python", "-", input=RUNTIME))
        # A second container gets no network. Only this verification process needs
        # scratch space to inspect its finalized PDF; HTTP rendering uses no files.
        offline = json.loads(docker("run", "--rm", "--interactive", "--name", offline_name,
                                    *LIMITS, "--network", "none", "--tmpfs", "/tmp:rw,noexec,nosuid,size=32m",
                                    image, "python", "-", input=OFFLINE))
        assert offline["pdf_sha256"] == receipt["pdf"]["sha256"]
        png = base64.b64decode(offline.pop("preview_base64"), validate=True)
        assert png == preview, "Container-native preview differs from the saved sample"
        (out / "invoice.png").write_bytes(png)
        receipt["offline_render"] = offline
        receipt["preview_sha256"] = hashlib.sha256(png).hexdigest()
        receipt["http"] = {"download": 200, "missing": 404, "post": 405,
                           "identical_parallel_downloads": 4, "home_and_preview_match": True}
        # Uvicorn receives SIGTERM as PID 1 and exits normally.
        docker("stop", "--time", "15", name)
        state = json.loads(docker("container", "inspect", name))[0]["State"]
        assert state["ExitCode"] == 0 and not state["OOMKilled"]
        receipt["normal_shutdown"] = True
        receipt["ok"] = True
    finally:
        if started:
            with (out / "server.log").open("w", encoding="utf-8") as log:
                subprocess.run(["docker", "logs", name], stdout=log, stderr=subprocess.STDOUT, timeout=30)
        cleanup = []
        for target in [name, offline_name]:
            result = subprocess.run(["docker", "container", "rm", "--force", target], capture_output=True, text=True)
            cleanup.append({"container": target, "removed_or_absent": result.returncode == 0 or "No such container" in result.stderr})
        if built:
            result = subprocess.run(["docker", "image", "rm", image], capture_output=True, text=True)
            cleanup.append({"image_tag": image, "removed": result.returncode == 0})
        receipt["cleanup"] = cleanup
        receipt["scope"] = "Actual local-container HTTP, offline render, runtime restrictions, independent PDF text/fonts, and native preview. Not a hosted deployment, capacity benchmark, or conformance claim."
        (out / "verification.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": receipt["ok"], "out": str(out), "pdf": receipt["pdf"]}))


if __name__ == "__main__":
    main()
