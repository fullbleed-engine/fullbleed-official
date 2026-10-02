#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check this document against its committed baseline, or record an intentional update."""
import argparse
from importlib.metadata import version
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
FONTS = ["Inter-Variable.ttf", "DMSerifDisplay-Regular.ttf",
         "DMSerifDisplay-Italic.ttf", "BebasNeue-Regular.ttf"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", nargs="?", choices=["check", "record"], default="check")
    args = parser.parse_args()
    expected = next(line.removeprefix("fullbleed==").strip()
                    for line in (ROOT / "requirements.txt").read_text().splitlines()
                    if line.startswith("fullbleed=="))
    installed = version("fullbleed")
    if installed != expected:
        raise SystemExit(f"This fixture requires Fullbleed {expected}; found {installed}. Install -r requirements.txt first.")
    out = ROOT / "output"
    out.mkdir(exist_ok=True)
    preview = out / "preview"
    # These filenames are owned by this example; preserve unrelated files.
    if preview.is_dir():
        for image in preview.glob("invoice_page*.png"):
            image.unlink()
    command = [sys.executable, "-I", "-m", "fullbleed", "--json-only", "render",
               "--html", "input.html", "--css", "style.css", "--out", "output/invoice.pdf",
               "--emit-image", "output/preview", "--image-dpi", "96",
               "--fail-on", "missing-glyphs", "--repro-record", "output/candidate.json"]
    for font in FONTS:
        command += ["--asset", "fonts/" + font]
    if args.action == "check":
        command += ["--repro-check", "baseline.json"]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    (out / "render.json").write_text(result.stdout, encoding="utf-8")
    (out / "stderr.txt").write_text(result.stderr, encoding="utf-8")
    if result.returncode == 0 and args.action == "record":
        report = json.loads(result.stdout)
        if not report["ok"]:
            raise SystemExit("The render did not pass; the baseline was not updated.")
        shutil.copyfile(out / "candidate.json", ROOT / "baseline.json")
        shutil.copyfile(out / "invoice.pdf", ROOT / "baseline.pdf")
    print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, file=sys.stderr, end="")
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
