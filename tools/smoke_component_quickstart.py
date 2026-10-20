# SPDX-License-Identifier: MIT
"""Run the exact component-alpha guide and installed agent examples."""
from __future__ import annotations

import argparse
from contextlib import closing
from hashlib import sha256
from html.parser import HTMLParser
from importlib import metadata
import json
from pathlib import Path
import re
import subprocess
import sys

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium

ROOT = Path(__file__).resolve().parents[1]


class Export(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.tags, self.inline = [], []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        self.tags.append(tag)
        if "style" in dict(attrs):
            self.inline.append(dict(attrs)["style"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    assert ROOT / "python" not in Path(fullbleed.__file__).parents, "Install the release wheel first"
    assert metadata.version("fullbleed") == args.expected_version
    example_source = "docs/component-bootstrap.md"
    section = (ROOT / example_source).read_text(encoding="utf-8").split("## Your first PDF and HTML exports\n", 1)[1].split("\n## ", 1)[0]
    blocks = re.findall(r"```python\n(.*?)```", section, re.S)
    assert len(blocks) == 1
    script = out / "order.py"
    script.write_text(blocks[0], encoding="utf-8", newline="\n")
    run = subprocess.run([sys.executable, "-I", str(script)], cwd=out, capture_output=True, text=True, encoding="utf-8", timeout=60)
    assert run.returncode == 0, (run.stdout, run.stderr)
    pdf = out / "order.pdf"
    reader = PdfReader(pdf)
    assert len(reader.pages) == 1 and reader.metadata.title == "Order summary"
    assert reader.trailer["/Root"]["/Lang"] == "en-US"
    content = "".join(reader.pages[0].extract_text().split())
    expected = ("Ready to ship", "ORDER / 1048", "Approved", "Your order has passed its final inspection.")
    assert all("".join(value.split()) in content for value in expected)
    with closing(pdfium.PdfDocument(pdf)) as document:
        with closing(document[0]) as page, closing(page.render(scale=1.5)) as bitmap:
            bitmap.to_pil().save(out / "order.pdfium.png")
    styled = (out / "order.html").read_text(encoding="utf-8")
    plain = (out / "order.unstyled.html").read_text(encoding="utf-8")
    assert "Bootstrap v5.0.0" in styled and "color: #205b47;" in styled
    assert "style" not in Export(plain).tags and not Export(plain).inline
    assert "script" not in Export(styled).tags and "script" not in Export(plain).tags
    assert "card-title" in plain and "Approved" in plain
    call = subprocess.run([sys.executable, "-I", "-m", "fullbleed", "agent-contract", "--format", "json"],
                          cwd=out, capture_output=True, text=True, encoding="utf-8", timeout=60)
    assert call.returncode == 0, call.stderr
    contract = json.loads(call.stdout)
    assert contract["product"]["version"] == args.expected_version
    example = next(item for item in contract["examples"] if item["id"] == "component_authoring_experimental_python")
    namespace = {}
    exec(compile(example["code"], "installed agent example", "exec"), namespace)
    assert "Bootstrap v5.0.0" in namespace["styled_html"] and "style" not in Export(namespace["plain_html"]).tags
    for name, value in (("agent-example.py", example["code"]), ("agent-styled.html", namespace["styled_html"]),
                        ("agent-unstyled.html", namespace["plain_html"])):
        (out / name).write_text(value, encoding="utf-8", newline="\n")
    # Preserve historical receipt fields for the example moved from the README;
    # identify its current source explicitly for new consumers.
    report = {"ok": True, "version": args.expected_version, "runtime": str(fullbleed.__file__),
              "example_source": example_source, "documentation_example_runs_alone": True,
              "readme_example_runs_alone": True, "readme_code_sha256": sha256(blocks[0].encode()).hexdigest(),
              "pdf_sha256": sha256(pdf.read_bytes()).hexdigest(), "pages": 1,
              "styled_and_unstyled_html": True, "bootstrap_javascript": False,
              "installed_agent_example_runs": True}
    (out / "verification.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "version": args.expected_version, "readme_and_installed_agent_examples": True}))


if __name__ == "__main__":
    main()
