# SPDX-License-Identifier: MIT
"""Check installed component calls and style contracts using test-only mypy."""
from __future__ import annotations

import argparse
from importlib import metadata
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import fullbleed.ui.experimental as components


VALID = '''from dataclasses import dataclass
from fullbleed.ui.experimental import (
    Column, DataTable, Document, Flow, Heading, Image, LayoutError, Node, Page, Paragraph,
    Row, Source, Stack, Style, Theme, component, css, mm, pt, style, styled, token,
)

@component(parts=("label", "note"))
def Notice(message: str, note: str | None = None) -> Node:
    return Paragraph(message, part="label")

valid: Node = Notice("Paid")
parts: tuple[str, ...] | None = Notice.parts

Label = styled(Paragraph, color=token("muted"), font_size=pt(8))
QuietNotice = styled(Notice, color="#334455")
typed: Node = QuietNotice("Paid").with_style(padding=pt(8))
override: Node = QuietNotice("Paid", style=style(color="blue"))
styled_parts: tuple[str, ...] | None = QuietNotice.parts
spacing = [pt(4), mm(2)]
recipe = Style(style(color="black"), padding=spacing)
root = Stack(Label("Invoice"), typed, gap=pt(6), align="start", style=recipe)
root = root.with_parts(label=style(font_weight=700)) if root.parts else root
flow = Flow([Paragraph("One"), Paragraph("Two")], (Paragraph(str(i)) for i in range(2)), None)

@dataclass
class Line:
    label: str
    amount: int

def description(row: Line) -> str:
    return row.label

rows = [Line("Design", 100)]
table = DataTable(rows, columns=[Column("Item", description)], row_break="avoid")
table = table.with_parts(cell=style(padding=(pt(4), pt(6))))
heading = Heading("Invoice", level=1, keep_with_next=True)
brand = Row(Paragraph("Fullbleed"), Image(src="mark.png", alt="Company mark"), justify="start")
document = Document(heading, brand, root, flow, table, theme=Theme({"muted": "#667580"}), page=Page("A5", margin=mm(12)))
compiled_html: str = document.to_html()
assert "Design" in compiled_html
diagnostic = LayoutError("Cannot fit", page=2, sources=document.compile().sources)
error_page: int = diagnostic.at("Batch").page
error_sources: tuple[Source, ...] = diagnostic.sources
escaped = css("transform: rotate(2deg)")
custom_variables = style({"--brand": "#315d9b"}, color="var(--brand)")
'''

INVALID_CASES = [
    ("component_argument", "Notice(10)"),
    ("component_keyword", 'Notice("Paid", unexpected=True)'),
    ("style_keyword", "style(paddding=pt(8))"),
    ("style_constructor_keyword", "Style(font_sze=pt(9))"),
    ("root_style_keyword", "valid.with_style(margn=pt(8))"),
    ("recipe_keyword", "styled(Paragraph, paddding=pt(8))"),
    ("style_value", "style(color=object())"),
    ("length_argument", 'pt("8")'),
    ("primitive_keyword", 'Paragraph("x", clss_name="item")'),
    ("primitive_class_type", 'Paragraph("x", class_name=10)'),
    ("layout_alignment", 'Stack(Paragraph("x"), align="around")'),
    ("layout_justification", 'Row(Paragraph("x"), justify="between")'),
    ("image_alt_required", 'Image(src="mark.png")'),
    ("document_page_type", 'Document(Paragraph("x"), page="A4")'),
    ("page_margin_type", "Page(margin=12)"),
    ("column_alignment", 'Column[str]("Item", str, align="start")'),
    ("column_record_type", 'DataTable(rows, columns=[Column[int]("Value", str)])'),
    ("table_repeat_flag", 'DataTable(rows, columns=[Column("Item", description)], repeat_header="yes")'),
    ("flow_text_requires_paragraph", 'Flow("raw text")'),
    ("styled_argument", "QuietNotice(123)"),
    ("styled_keyword", 'QuietNotice("Paid", unexpected=True)'),
]

# Recursive Iterable types also admit strings. Preserve valid custom iterables
# rather than narrowing the API just to make this negative example type-check.
# Record whether each checker catches it; runtime validation is mandatory.
RUNTIME_ONLY_CASES = {"flow_text_requires_paragraph"}


def invalid_consumer():
    source = VALID + "\n"
    expected = {}
    for label, statement in INVALID_CASES:
        expected[source.count("\n") + 1] = label
        source += statement + "\n"
    return source, expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[1] / "python"
    assert source not in Path(components.__file__).parents, "Use an installed candidate wheel with python -I"
    assert Path(components.__file__).with_name("py.typed").is_file(), "The wheel must carry its typing marker"
    for name in ("_style", "_tree", "_table", "_document"):
        assert Path(components.__file__).with_name(name + ".pyi").is_file(), name
    environment = dict(os.environ)
    environment.pop("MYPYPATH", None)
    environment.pop("PYTHONPATH", None)
    with tempfile.TemporaryDirectory(prefix="fullbleed-component-types-") as directory:
        folder = Path(directory)
        (folder / "valid.py").write_text(VALID, encoding="utf-8")
        invalid_source, expected = invalid_consumer()
        (folder / "invalid.py").write_text(invalid_source, encoding="utf-8")

        def check(name):
            return subprocess.run(
                [sys.executable, "-I", "-m", "mypy", "--strict", "--python-version=3.10", "--follow-imports=silent", "--no-incremental", name],
                cwd=folder, env=environment, capture_output=True, text=True, timeout=60,
            )

        runtime = subprocess.run([sys.executable, "-I", "valid.py"], cwd=folder, env=environment,
                                 capture_output=True, text=True, timeout=20)
        assert runtime.returncode == 0, runtime.stdout + runtime.stderr
        valid, invalid = check("valid.py"), check("invalid.py")
        assert valid.returncode == 0, valid.stdout + valid.stderr
        assert invalid.returncode == 1, invalid.stdout + invalid.stderr
        actual = [int(line) for line in re.findall(r"invalid\.py:(\d+): error:", invalid.stdout)]
        missed = {label for line, label in expected.items() if line not in actual}
        assert not missed - RUNTIME_ONLY_CASES and not set(actual) - set(expected), (missed, invalid.stdout)
        try:
            components.Flow("raw text")
        except components.AuthoringError as error:
            assert error.code == "flow.child", error
            runtime_guard = error.to_dict()
        else:
            raise AssertionError("Flow must reject unwrapped text at runtime")
    result = {"ok": True, "mypy_version": metadata.version("mypy"), "runtime": str(components.__file__),
              "valid_consumer": valid.stdout, "valid_consumer_runs": True,
              "expected_invalid_consumer": invalid.stdout, "invalid_cases": expected,
              "rejected_cases": len(expected) - len(missed), "runtime_only_cases": sorted(missed),
              "flow_runtime_guard": runtime_guard,
              "scope": "Installed calls, style keywords and value shapes, page options, primitive props, layout choices, table record types and preserved component arguments. Finite values, units, CSS syntax, token names and dynamic part names still require runtime validation. The extra style keyword on a styled custom factory has a permissive argument fallback; use node.with_style for full argument checking."}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rejected_cases": len(expected) - len(missed), "runtime_only_cases": sorted(missed)}))


if __name__ == "__main__":
    main()
