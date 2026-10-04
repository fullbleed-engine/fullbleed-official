#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build the standalone source ZIP and its actual PDF/PNG sample assets."""
import argparse
from datetime import datetime, timezone
import hashlib
from importlib import metadata
import json
from pathlib import Path
import subprocess
import zipfile

import fullbleed
from invoice import load_invoice, render_invoice

ROOT = Path(__file__).resolve().parent
REPOSITORY = ROOT.parents[1]
FILES = ["README.md", "invoice.py", "demo.py", "fastapi_app.py", "flask_app.py", "django_app.py",
         "check_examples.py", "check_http.py", "requirements-fastapi.txt", "requirements-flask.txt",
         "requirements-django.txt", "requirements-check.txt", "templates/invoice.html",
         "templates/invoice.css", "static/index.html", "static/invoice.png"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--", str(ROOT)], cwd=REPOSITORY, text=True).strip()
    if dirty:
        raise SystemExit("Commit the example source before packaging its public download.")
    out.mkdir(parents=True, exist_ok=False)
    # Package committed bytes, independent of checkout newline conversion.
    def committed(path: Path) -> bytes:
        return subprocess.check_output(["git", "show", source_commit+":"+path.relative_to(REPOSITORY).as_posix()], cwd=REPOSITORY)
    content = {name: committed(ROOT/name) for name in FILES}
    for path in sorted((ROOT/"fonts").iterdir()):
        if path.is_file(): content[path.relative_to(ROOT).as_posix()] = committed(path)
    content["LICENSE"] = committed(REPOSITORY/"LICENSE")
    content[".gitignore"] = b".venv/\n__pycache__/\noutput/\n"
    pdf = render_invoice(load_invoice("INV-1042"))
    (out/"invoice.pdf").write_bytes(pdf)
    previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages_to_dir(
        str(out/"invoice.pdf"), str(out/"preview"), 110, "invoice")
    assert len(previews) == 1
    png = Path(previews[0]).read_bytes()
    assert png == content["static/invoice.png"], "Refresh and review the saved preview before packaging."
    (out/"invoice.png").write_bytes(png)
    digest = lambda data: hashlib.sha256(data).hexdigest()
    manifest = {"schema": "fullbleed.python-web-starter.v1", "engineVersion": metadata.version("fullbleed"),
                "sourceRepository": "https://github.com/fullbleed-engine/fullbleed-official",
                "sourceCommit": source_commit,
                "files": [{"path": name, "bytes": len(data), "sha256": digest(data)} for name,data in sorted(content.items())]}
    content["MANIFEST.json"] = (json.dumps(manifest, indent=2)+"\n").encode()
    archive = out/"project.zip"
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as zipped:
        for name,data in sorted(content.items()):
            item = zipfile.ZipInfo("fullbleed-python-invoice/"+name, (2026,1,1,0,0,0))
            item.compress_type = zipfile.ZIP_DEFLATED
            item.external_attr = 0o100644 << 16
            zipped.writestr(item,data)
    manifest["checkedAt"] = datetime.now(timezone.utc).isoformat()
    manifest["assets"] = [{"file": name, "bytes": (out/name).stat().st_size,
                            "sha256": digest((out/name).read_bytes())} for name in ["project.zip","invoice.pdf","invoice.png"]]
    (out/"source.json").write_text(json.dumps(manifest,indent=2)+"\n",encoding="utf-8")
    print(json.dumps({"sourceCommit":source_commit,"assets":manifest["assets"]}))


if __name__ == "__main__": main()
