"""Authored preformatted wrapping must keep saved PDF text on the page."""
from importlib.resources import files
from io import BytesIO
import json

import fullbleed
from pypdf import PdfReader
import pypdfium2 as pdfium
import pytest


def engine():
    font = files("fullbleed_assets").joinpath("fonts", "NotoSans-Regular.ttf")
    return fullbleed.PdfEngine(font_files=[str(font)])


def stylesheet(policy):
    return """@page { size:180pt 120pt; margin:12pt; }
body { margin:0; }
pre, code { font-family:'Noto Sans'; font-size:10pt; line-height:14pt; }
code { white-space:inherit; overflow-wrap:inherit; word-break:inherit; }
pre { margin:0; %s }
""" % policy


def render(renderer, path, text, css):
    if path == "direct":
        return bytes(renderer.render_pdf(f"<pre><code>{text}</code></pre>", css))
    if path == "compiled":
        return bytes(renderer.compile_pdf(f"<pre><code>{text}</code></pre>", css).render_pdf())
    return bytes(renderer.compile_pdf("<pre><code>{{content}}</code></pre>", css)
                 .render_pdf_reflow_bindings({"content": [text]}))


def inspect(data, tmp_path):
    (tmp_path / "document.pdf").write_bytes(data)
    reader = PdfReader(BytesIO(data), strict=True)
    text = "\n".join(page.extract_text() for page in reader.pages)
    characters = []
    pdf = pdfium.PdfDocument(data)
    try:
        for index in range(len(pdf)):
            page = pdf[index]
            textpage = page.get_textpage()
            for offset in range(textpage.count_chars()):
                character = textpage.get_text_range(offset, 1)
                if character.strip():
                    box = textpage.get_charbox(offset)
                    if box[2] > box[0] and box[3] > box[1]:
                        characters.append((index, character, box))
            bitmap = page.render(scale=2)
            bitmap.to_pil().save(tmp_path / f"page-{index + 1}.png")
            bitmap.close()
            textpage.close()
            page.close()
    finally:
        pdf.close()
    (tmp_path / "characters.json").write_text(json.dumps(characters), encoding="utf-8")
    assert characters, "The PDF must contain visible text"
    return len(reader.pages), text, characters


def compact(text):
    return "".join(text.split())


@pytest.mark.parametrize("path", ["direct", "compiled", "reflow"])
@pytest.mark.parametrize("policy,body", [
    ("white-space:pre-wrap; overflow-wrap:anywhere", "abcdefghij" * 60),
    ("white-space:pre-wrap; overflow-wrap:break-word", "abcdefghij" * 60),
    ("white-space:pre-wrap; word-break:break-all", "abcdefghij" * 60),
    ("white-space:pre-line; overflow-wrap:anywhere", "abcdefghij" * 60),
    ("white-space:break-spaces; overflow-wrap:anywhere", "abcdefghij" * 60),
    ("white-space:normal; overflow-wrap:anywhere", "abcdefghij" * 60),
    ("white-space:pre-wrap", "word " * 100),
], ids=["anywhere", "break-word", "break-all", "pre-line", "break-spaces", "normal", "words"])
def test_preformatted_wrapping_stays_inside_page(tmp_path, path, policy, body):
    renderer = engine()
    text = f"   PREFIX {body}\n    SUFFIX"
    css = stylesheet(policy)
    data = render(renderer, path, text, css)
    assert data == render(renderer, path, text, css), "Rendering must be repeatable"
    pages, actual, characters = inspect(data, tmp_path)
    assert pages > 1, "The long preformatted block must wrap and paginate"
    assert compact(actual) == compact(text), "All authored text must appear once, in order"
    outside = [(page, char, box) for page, char, box in characters
               if box[0] < 11 or box[2] > 169 or box[1] < 11 or box[3] > 109]
    assert not outside, f"Visible glyphs outside the print area: {outside[:10]}"


@pytest.mark.parametrize("policy", [
    "overflow-wrap:anywhere",
    "white-space:pre; overflow-wrap:anywhere",
    "white-space:nowrap; overflow-wrap:anywhere",
    "white-space:pre-wrap; text-wrap:nowrap; overflow-wrap:anywhere",
])
def test_preformatted_no_wrap_policy_is_preserved(tmp_path, policy):
    text = "PREFIX " + "abcdefghij" * 60
    data = render(engine(), "direct", text, stylesheet(policy))
    pages, actual, characters = inspect(data, tmp_path)
    assert pages == 1
    assert compact(actual) == compact(text)
    assert any(box[2] > 180 for _, _, box in characters), "No-wrap intentionally overflows"


@pytest.mark.parametrize("mode,preserve_indent,preserve_newline", [
    ("pre", True, True),
    ("pre-wrap", True, True),
    ("break-spaces", True, True),
    ("pre-line", False, True),
    ("normal", False, False),
    ("nowrap", False, False),
])
def test_preformatted_whitespace_follows_authored_policy(
        tmp_path, mode, preserve_indent, preserve_newline):
    data = render(engine(), "direct", "   ALPHA\n    BETA", stylesheet(f"white-space:{mode}"))
    _, actual, characters = inspect(data, tmp_path)
    assert compact(actual) == "ALPHABETA"
    first = next(box for _, char, box in characters if char == "A")
    second = next(box for _, char, box in characters if char == "B")
    assert (first[0] > 17) == preserve_indent
    assert (first[1] - second[1] > 10) == preserve_newline
