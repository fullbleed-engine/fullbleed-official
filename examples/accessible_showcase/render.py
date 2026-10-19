#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Render and export the accessible showcase using only Fullbleed and stdlib."""

from __future__ import annotations

import argparse
import hashlib
from importlib import metadata, resources
import json
from pathlib import Path
import shutil
import subprocess
import sys

import fullbleed
from fullbleed.ui.experimental import Renderer
from guide import ROOT, build_document


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class GuideRenderer(Renderer):
    """Register the same named illustration for each isolated engine."""

    def engine_for(self, document):
        engine = super().engine_for(document)
        assets = fullbleed.AssetBundle()
        assets.add_file(str(ROOT / "garden.svg"), "svg", name="garden.svg")
        engine.register_bundle(assets)
        return engine


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "output")
    parser.add_argument("--pdf-profile", choices=("pdfua1", "pdfua2"), default="pdfua1",
                        help="PDF/UA profile for both component and HTML/CSS output (default: pdfua1)")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    inter = Path(str(resources.files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")))
    serif = ROOT / "fonts/DMSerifDisplay-Regular.ttf"
    fonts = (inter, serif)
    document = build_document()
    source = document.compile(a11y_mode="raise", target="pdf")
    renderer = GuideRenderer(font_files=[str(font) for font in fonts], pdf_profile=args.pdf_profile)
    prepared = document.prepare(renderer, a11y_mode="raise")
    pdf = out / "common-ground-guide.pdf"
    pdf.write_bytes(prepared.render_pdf())
    replay = document.render_pdf(renderer, a11y_mode="raise")
    if replay != pdf.read_bytes():
        raise RuntimeError("PDF replay was not byte-for-byte deterministic")
    (out / "source.html").write_text(source.html, encoding="utf-8", newline="\n")
    (out / "source.css").write_text(source.css, encoding="utf-8", newline="\n")
    browser_document = build_document(browser=True)
    for styled, filename in ((True, "guide.html"), (False, "guide-unstyled.html")):
        (out / filename).write_text(browser_document.to_html(include_styles=styled, a11y_mode="raise"),
                                   encoding="utf-8", newline="\n")
    shutil.copyfile(ROOT / "garden.svg", out / "garden.svg")
    (out / "fonts").mkdir(exist_ok=True)
    for font in fonts:
        shutil.copyfile(font, out / "fonts" / font.name)
    for license_file in (ROOT / "fonts").glob("*-OFL.txt"):
        shutil.copyfile(license_file, out / "fonts" / license_file.name)
    engine = renderer.engine_for(document)
    previews = engine.render_finalized_pdf_image_pages_to_dir(str(pdf), str(out / "preview"), 144, "guide")
    inspection = fullbleed.inspect_pdf(str(pdf))
    for name, value in (("a11y-source.json", source.a11y_report),
                        ("inspection.json", inspection),
                        ("pagination.json", prepared.export_render_time_pagination_trace())):
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    # CLI diagnostics independently exercise the exported HTML/CSS authoring path.
    command = [sys.executable, "-I", "-m", "fullbleed", "--json-only", "verify",
               "--html", str(out / "source.html"), "--css", str(out / "source.css"),
               "--document-title", document.title, "--document-lang", document.lang,
               "--pdf-profile", args.pdf_profile, "--emit-pdf", str(out / "verified.pdf"),
               "--fail-on", "overflow", "--fail-on", "missing-glyphs", "--fail-on", "font-subst"]
    for asset in (*fonts, ROOT / "garden.svg"):
        command.extend(("--asset", str(asset)))
    checked = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
    (out / "verify.json").write_text(checked.stdout, encoding="utf-8")
    (out / "verify.stderr.txt").write_text(checked.stderr, encoding="utf-8")
    verification = json.loads(checked.stdout)
    parity = sha(pdf) == sha(out / "verified.pdf") if (out / "verified.pdf").exists() else False
    result = dict(schema="fullbleed.accessible_showcase.v1",
                  ok=checked.returncode == 0 and bool(verification.get("ok")) and parity
                     and inspection.get("page_count") == 2,
                  runtime=metadata.version("fullbleed"), pdf_profile=args.pdf_profile,
                  pdf=pdf.name, sha256=sha(pdf),
                  page_count=inspection.get("page_count"), deterministic_replay=True,
                  html_css_pdf_byte_parity=parity, native_diagnostic_gate=verification.get("ok"),
                  previews=[Path(path).relative_to(out).as_posix() for path in previews],
                  fonts=[dict(file=font.name, sha256=sha(font)) for font in fonts],
                  external_pdfua_validation="Run veraPDF separately; see README.md.")
    (out / "render-report.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result), flush=True)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
