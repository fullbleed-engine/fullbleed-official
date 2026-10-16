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
    Column, DataTable, Document, DocumentSource, Flow, Grid, Heading, Image, LayoutError, Node, Page, Paragraph,
    Renderer, Row, Source, Stack, Style, Theme, component, css, mm, pt, style, styled, to_legacy, token,
)

@component(parts=("label", "note"))
def Notice(message: str, note: str | None = None) -> Node:
    return Paragraph(message, part="label")

valid: Node = Notice("Paid")
parts: tuple[str, ...] | None = Notice.parts

Label = styled(Paragraph, color=token("muted"), font_size=pt(8))
QuietNotice = styled(Notice, color="#334455")
typed: Node = QuietNotice("Paid").with_style(padding=pt(8))
override: Node = QuietNotice("Paid").with_style(color="blue")
primitive_override: Node = Label("Paid", style=style(color="blue"))
styled_parts: tuple[str, ...] | None = QuietNotice.parts

@component
def Choice(*, style: str) -> Node:
    return Paragraph(style)

StyledChoice = styled(Choice, color="purple")
business_style: Node = StyledChoice(style="formal")
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
SpacedFlow = styled(Flow, row_gap=pt(12))
AlignedRow = styled(Row, align_items="end", justify_content="end")
SplitGrid = styled(Grid, grid_template_columns="2fr 1fr")
KeptHeading = styled(Heading, break_after="avoid")
optional_flow = SpacedFlow(Paragraph("Choice"), gap=None)
explicit_flow = SpacedFlow(Paragraph("Choice"), gap=0)
optional_stack = Stack(Paragraph("Choice"), gap=None, align=None)
explicit_row = AlignedRow(Paragraph("Choice"), gap=None, align="start", justify=None)
optional_grid = SplitGrid(Paragraph("Choice"), columns=None, gap=None)
explicit_grid = SplitGrid(Paragraph("Choice"), columns=3, gap=pt(3))
optional_heading = KeptHeading("Choice", keep_with_next=None)
explicit_heading = KeptHeading("Choice", keep_with_next=False)
document = Document(heading, brand, root, flow, table, theme=Theme({"muted": "#667580"}), page=Page("A5", margin=mm(12)))
requirements: tuple[str, ...] = root.required_tokens
document_requirements: tuple[str, ...] = document.required_tokens
brand_theme: Theme = document.theme.with_values({"muted": "#112233"}, accent="#315d9b")
compiled_html: str = document.to_html()
assert "Design" in compiled_html
compiled_source: DocumentSource = document.compile(a11y_mode="raise")
assert compiled_source.a11y_report is not None

def prepare_outputs(document: Document, renderer: Renderer) -> tuple[bytes, list[bytes]]:
    prepared = document.prepare(renderer, a11y_mode="raise")
    pdf: bytes = prepared.render_pdf()
    previews: list[bytes] = prepared.render_image_pages(dpi=96)
    return pdf, previews
diagnostic = LayoutError("Cannot fit", page=2, sources=document.compile().sources)
error_page: int = diagnostic.at("Batch").page
error_sources: tuple[Source, ...] = diagnostic.sources
failure_reason: str | None = diagnostic.reason
failure_sources: tuple[Source, ...] = diagnostic.at("Batch").failure_sources
legacy_children = to_legacy(root, theme=Theme({"muted": "#667580"}))
assert len(legacy_children) == 1
escaped = css("transform: rotate(2deg)")
custom_variables = style({"--brand": "#315d9b"}, color="var(--brand)")
'''

INVALID_CASES = [
    ("source_is_not_layout", "compiled_source.render_pdf()"),
    ("semantic_policy", 'document.compile(a11y_mode="strict")'),
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
    ("heading_keep_type", 'Heading("x", keep_with_next="yes")'),
    ("grid_columns_type", 'Grid(Paragraph("x"), columns=object())'),
    ("styled_layout_alignment", 'AlignedRow(Paragraph("x"), align="around")'),
    ("styled_heading_keep_type", 'KeptHeading("x", keep_with_next="yes")'),
    ("image_alt_required", 'Image(src="mark.png")'),
    ("document_page_type", 'Document(Paragraph("x"), page="A4")'),
    ("page_margin_type", "Page(margin=12)"),
    ("column_alignment", 'Column[str]("Item", str, align="start")'),
    ("column_record_type", 'DataTable(rows, columns=[Column[int]("Value", str)])'),
    ("table_repeat_flag", 'DataTable(rows, columns=[Column("Item", description)], repeat_header="yes")'),
    ("flow_text_requires_paragraph", 'Flow("raw text")'),
    ("styled_argument", "QuietNotice(123)"),
    ("styled_keyword", 'QuietNotice("Paid", unexpected=True)'),
    ("styled_extra_style", 'QuietNotice("Paid", style=style(color="blue"))'),
    ("styled_primitive_class_with_style", 'Label("Paid", class_name=10, style=style(color="blue"))'),
    ("styled_business_style_type", 'StyledChoice(style=10)'),
    ("migration_node", 'to_legacy("<p>HTML</p>")'),
    ("migration_theme", 'to_legacy(valid, theme={"muted": "gray"})'),
    ("preparation_renderer", 'document.prepare("default")'),
    ("prepared_preview_dpi", 'document.prepare(Renderer()).render_image_pages(dpi="96")'),
    ("prepared_pdf_input", 'document.prepare(Renderer()).render_pdf(html="<p>New</p>")'),
    ("theme_update_mapping", 'document.theme.with_values([("muted", "red")])'),
    ("theme_update_value", 'document.theme.with_values(accent=object())'),
    ("node_tokens_readonly", 'root.required_tokens = ("invented",)'),
    ("document_tokens_readonly", 'document.required_tokens = ("invented",)'),
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
    for name in ("_style", "_tree", "_table", "_document", "_bridge"):
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
              "scope": "Installed calls, style keywords and value shapes, page options, primitive props, layout choices, table record types and preserved component arguments. Styled factories retain exactly their wrapped argument types, including business arguments named style. Finite values, units, CSS syntax, token names and dynamic part names still require runtime validation."}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rejected_cases": len(expected) - len(missed), "runtime_only_cases": sorted(missed)}))


if __name__ == "__main__":
    main()
