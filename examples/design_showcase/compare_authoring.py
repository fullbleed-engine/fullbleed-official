#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Render the original HTML and component versions; require identical page PNGs.

Run with the installed Fullbleed wheel and Python's standard library only.
The original templates, data, CSS and explicit font assets are the reference.
"""
from __future__ import annotations

import argparse
from decimal import Decimal
from hashlib import sha256
from html.parser import HTMLParser
from importlib.metadata import version
import json
from pathlib import Path
import sys

from fullbleed.ui.experimental import Renderer

import render as original
from invoice_components import northstar_invoice
from report_components import common_ground_report


class ExportCheck(HTMLParser):
    def __init__(self):
        super().__init__()
        self.styles = self.inline_styles = self.scripts = 0
        self.classes = []

    def handle_starttag(self, tag, attrs):
        self.styles += tag == "style"
        self.scripts += tag == "script"
        self.inline_styles += "style" in dict(attrs)
        self.classes.extend(name for name in dict(attrs).get("class", "").split() if not name.startswith("fbx-"))


def digest(path: Path) -> str:
    return sha256(path.read_bytes()).hexdigest()


def compare(out: Path, name: str, *, dpi: int) -> dict:
    if name == "invoice":
        items = original.DATA["invoice"]
        document = northstar_invoice(items)
        total = sum((Decimal(item["rate"]) * item["hours"] for item in items), Decimal(0))
        expected_pages, markers = 1, ["NS-1042", "Maple & Finch", original.money(total)]
    else:
        report = original.DATA["report"]
        document = common_ground_report(report)
        total = sum(Decimal(item["amount"]) for item in report["allocations"])
        expected_pages, markers = 3, ["Common Ground", f"{report['households']:,}", original.money(total), "What we count."]
    source = document.compile(target="pdf")
    html, css = original.sources(name)
    results = {}
    for lane, text, rules in (("html", html, css), ("components", source.html, source.css)):
        results[lane] = original.ordinary(out / lane, name, text, rules, expected_pages, markers, dpi=dpi)
    failures = [f"{lane}: {failure}" for lane, result in results.items() for failure in result["failures"]]
    left, right = (out / lane / name for lane in ("html", "components"))
    pages = []
    for index in range(1, expected_pages + 1):
        filename = f"{name}_page{index}.png"
        a, b = left / "preview" / filename, right / "preview" / filename
        equal = a.exists() and b.exists() and a.read_bytes() == b.read_bytes()
        pages.append(dict(page=index, identical=equal, html_sha256=digest(a) if a.exists() else None,
                          component_sha256=digest(b) if b.exists() else None))
        if not equal:
            failures.append(f"page {index}: authoring paths produce different PNGs")
    exports = {}
    for label, included in (("styled", True), ("unstyled", False)):
        text = document.to_html(include_styles=included)
        (right / f"{name}.{label}.html").write_text(text, encoding="utf-8", newline="\n")
        check = ExportCheck()
        check.feed(text)
        exports[label] = check
        if check.scripts or not included and (check.styles or check.inline_styles):
            failures.append(f"{label}: unexpected script or styles")
    classes_preserved = exports["styled"].classes == exports["unstyled"].classes
    if not classes_preserved:
        failures.append("unstyled export changed authored class names")
    if not exports["styled"].styles:
        failures.append("styled export omitted its CSS")
    a, b = left / f"{name}.pdf", right / f"{name}.pdf"
    renderer = Renderer(font_files=[str(path) for path in original.FONTS])
    api_pdf = document.render_pdf(renderer)
    styled_pdf = renderer.engine_for(document).render_pdf(document.to_html(include_styles=True), "")
    api_equal = b.exists() and api_pdf == b.read_bytes()
    styled_equal = b.exists() and styled_pdf == b.read_bytes()
    if not api_equal or not styled_equal:
        failures.append("direct component API or embedded-style HTML changed PDF bytes")
    return dict(name=name, ok=not failures, failures=failures, page_count=expected_pages,
                html_pdf_sha256=digest(a) if a.exists() else None,
                component_pdf_sha256=digest(b) if b.exists() else None,
                pdf_bytes_identical=a.exists() and b.exists() and a.read_bytes() == b.read_bytes(),
                pages=pages, render_verify_replay=all(result["ok"] for result in results.values()),
                direct_component_api_pdf_identical=api_equal, styled_html_pdf_identical=styled_equal,
                styled_and_unstyled_exports=True, authored_classes_preserved=classes_preserved)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=original.ROOT / "output" / "authoring-comparison")
    parser.add_argument("--only", choices=("invoice", "report"))
    parser.add_argument("--dpi", type=int, default=120)
    args = parser.parse_args()
    if args.dpi <= 0:
        parser.error("--dpi must be positive")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    records = [compare(out, name, dpi=args.dpi) for name in ([args.only] if args.only else ["invoice", "report"])]
    result = dict(ok=all(record["ok"] for record in records), engine_version=version("fullbleed"),
                  dpi=args.dpi, documents=records,
                  fonts={path.name: digest(path) for path in original.FONTS},
                  scope="Same-runtime PDF page appearance for these fixed A4 compositions; not general HTML/CSS parity.")
    (out / "comparison.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps(result, indent=2))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
