"""Complete authored text survives grid pagination in saved PDF output.

End markers alone cannot detect an interior word being lost or repeated. These
checks compare every non-whitespace character, in order within each column,
with the authored data. They make no claim about whitespace extraction, reading
order across columns, visual parity or PDF tagging.
"""

from html import escape
from importlib.resources import files
from io import BytesIO
import re

import fullbleed
from pypdf import PdfReader
import pytest


ROWS = [
    (
        f"ASSET-REFERENCE-{row:02d}-LONG-WRAPPABLE LABEL-END-{row:02d}",
        " ".join(f"record{row:02d}word{word:02d}" for word in range(1, 21))
        + f". Quantity: {row + 10}; USD {row},950.00. VALUE-END-{row:02d}",
    )
    for row in range(1, 5)
] + [
    (
        "VERIFY",
        "https://example.invalid/records/very/long/verification/path/FINAL-VERIFICATION-END",
    )
]


def _without_whitespace(text):
    # Long URLs may wrap inside a word. Keep every authored punctuation mark,
    # including line-ending hyphens; ignore only extraction whitespace.
    return re.sub(r"\s+", "", text)


def _column_text(reader, width):
    columns = [[], []]
    # The first track fills fit-content's 30% limit. The divider is halfway
    # through the 16px gap, converted from CSS pixels to PDF points.
    divider = (24 + (width - 48) * 0.3 + 8) * 0.75

    def collect(text, cm, tm, font, size):
        # Transform the text origin into page coordinates. Compiled PDFs may
        # use a different transformation matrix from direct output.
        x = tm[4] * cm[0] + tm[5] * cm[2] + cm[4]
        columns[int(x >= divider)].append(text)

    for page in reader.pages:
        page.extract_text(visitor_text=collect)
    return [_without_whitespace("".join(parts)) for parts in columns]


@pytest.mark.parametrize("width,height", [(624, 360), (480, 280)])
@pytest.mark.parametrize("mode", ["direct", "compiled", "repeated", "reflow", "compact-reflow"])
def test_paginated_contents_grid_preserves_complete_column_text(tmp_path, width, height, mode):
    engine = fullbleed.PdfEngine(
        font_files=[str(files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf"))]
    )
    body = '<h1>Inventory batch {{batch}}</h1><div class="grid">' + "".join(
        '<div class="row"><div class="label">' + escape(label)
        + '</div><div class="value">{{batch}}: ' + escape(value) + "</div></div>"
        for label, value in ROWS
    ) + "</div>"
    html = '<html lang="en"><body>' + body + "</body></html>"
    css = f"@page {{ size: {width}px {height}px; margin: 24px }}" + """
        * { margin: 0; box-sizing: border-box }
        body { font: 16px/1.25 Inter; color: #111; background: white }
        h1 { margin-bottom: 12px; padding: 8px; font-size: 24px }
        .grid { display: grid; grid-template-columns: fit-content(30%) minmax(0, 1fr);
                gap: 8px 16px; width: 100% }
        .row { display: contents }
        .label, .value { padding: 8px; border: 3px solid #111; overflow-wrap: anywhere }
        .label { color: white; background: #c1121f; font-weight: 700 }
        .value { background: #a8dadc }
    """
    (tmp_path / "source.html").write_text(html, encoding="utf-8")
    (tmp_path / "source.css").write_text(css, encoding="utf-8")
    literal = html.replace("{{batch}}", "Alpha")
    direct = bytes(engine.render_pdf(literal, css))
    direct_pages = len(PdfReader(BytesIO(direct)).pages)
    assert direct_pages >= 3, "The fixture must cross at least two page breaks"
    batches = ["Alpha"]
    if mode == "direct":
        data = direct
    elif mode == "compiled":
        data = bytes(engine.compile_pdf(literal, css).render_pdf())
    elif mode == "repeated":
        batches = ["Alpha", "Alpha"]
        data = bytes(engine.compile_pdf(literal, css).render_pdf_batch(2))
    else:
        batches = ["Alpha", "Bravo"]
        options = {"compression": "compact"} if mode == "compact-reflow" else {}
        data = bytes(engine.compile_pdf(html, css).render_pdf_reflow_bindings(
            {"batch": batches}, **options
        ))
    (tmp_path / f"{mode}.pdf").write_bytes(data)
    reader = PdfReader(BytesIO(data))
    labels = " ".join(label for label, _ in ROWS)
    expected_left = " ".join(f"Inventory batch {batch} {labels}" for batch in batches)
    expected_right = " ".join(f"{batch}: {value}" for batch in batches for _, value in ROWS)
    assert len(reader.pages) == direct_pages * len(batches)
    actual_left, actual_right = _column_text(reader, width)
    assert actual_left == _without_whitespace(expected_left)
    assert actual_right == _without_whitespace(expected_right)
