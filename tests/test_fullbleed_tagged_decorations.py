"""Inspect emitted marking boundaries, independently of the engine's inspector."""
import io

import fullbleed
import fullbleed_assets
from pypdf import PdfReader
import pytest


HTML = """<main><h1>Notice</h1>
<p class="decorated">Welcome <span class="small">{{name}}</span> to the workshop.</p>
<p>The next paragraph remains real content.</p></main>"""
CSS = """@page { size: 300pt 300pt; margin: 20pt; }
body { font-family: Inter; font-size: 12pt; }
.small { font-size: 9pt; }
.decorated { border-top: 3pt solid #183c38; border-bottom: 2pt solid #183c38;
padding: 8pt; background: #eef2e5; }"""


@pytest.mark.parametrize("profile", ["tagged", "pdfua1", "pdfua2"])
@pytest.mark.parametrize("mode", ["direct", "compiled", "repeat", "reflow", "compact"])
def test_decorated_inline_groups_keep_paint_marked_and_text_real(profile, mode):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile,
        document_lang="en-US",
        document_title="Tagged grouping decoration",
        font_files=[str(fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"))],
    )
    if mode == "direct":
        pdf = engine.render_pdf(HTML.replace("{{name}}", "Ada"), CSS)
    else:
        compiled = engine.compile_pdf(
            HTML if mode in {"reflow", "compact"} else HTML.replace("{{name}}", "Ada"), CSS
        )
        if mode == "compiled":
            pdf = compiled.render_pdf()
        elif mode == "repeat":
            pdf = compiled.render_pdf_batch(2)
        else:
            pdf = compiled.render_pdf_reflow_bindings(
                {"name": ["Ada", "Bea"]},
                compression="compact" if mode == "compact" else "throughput",
            )
    reader = PdfReader(io.BytesIO(pdf))
    assert len(reader.pages) == (1 if mode in {"direct", "compiled"} else 2)
    assert "/StructTreeRoot" in reader.trailer["/Root"]
    for page in reader.pages:
        marks = []
        paints = artifacts = grouping_paragraphs = 0
        for operands, operator in page.get_contents().operations:
            if operator in {b"BDC", b"BMC"}:
                artifact = operands[0] == "/Artifact"
                has_mcid = operator == b"BDC" and "/MCID" in operands[1]
                marks.append((artifact, has_mcid))
                artifacts += int(artifact)
                grouping_paragraphs += int(operator == b"BMC" and operands[0] == "/P")
            elif operator == b"EMC":
                assert marks, "Unbalanced marked-content close"
                marks.pop()
            elif operator in {b"Tj", b"TJ", b"'", b'"', b"Do", b"S", b"s", b"f", b"F", b"f*", b"B", b"B*", b"b", b"b*", b"sh"}:
                paints += 1
                assert any(artifact or mcid for artifact, mcid in marks), (
                    "Paint inside a grouping tag still needs MCID or Artifact coverage", operator, marks
                )
                if operator in {b"Tj", b"TJ", b"'", b'"'}:
                    assert not any(artifact for artifact, _ in marks), "Real notice text was hidden in an artifact"
        assert not marks
        assert paints > 0 and artifacts > 0 and grouping_paragraphs > 0
