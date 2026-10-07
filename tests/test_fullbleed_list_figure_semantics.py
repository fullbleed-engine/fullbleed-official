"""Read the final structure tree rather than trusting profile metadata."""
import io

import fullbleed
import fullbleed_assets
from pypdf import PdfReader
import pytest


MODES = ["direct", "compiled", "repeat", "reflow", "compact"]
PROFILES = ["tagged", "pdfua1", "pdfua2"]
CSS = """@page { size: 400pt 600pt; margin: 24pt; }
body { font-family: Inter; font-size: 10pt; }
ul, ol, li, p, figure, figcaption { margin: 0; }
ul, ol { padding-left: 20pt; }
figure { padding: 4pt; border: 1pt solid #183c38; }
img { width: 40pt; height: 20pt; }
"""
IMAGE = ('data:image/svg+xml,'
         '%3Csvg xmlns="http://www.w3.org/2000/svg" width="40" height="20"%3E'
         '%3Crect width="40" height="20" fill="green"/%3E%3C/svg%3E')


def render(html, profile, mode="direct", css=CSS):
    engine = fullbleed.PdfEngine(
        pdf_profile=profile, document_lang="en-US",
        document_title="Lists and captions",
        font_files=[str(fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"))],
    )
    if mode == "direct":
        pdf = engine.render_pdf(html.replace("{{name}}", "Ada"), css)
    else:
        compiled = engine.compile_pdf(
            html if mode in {"reflow", "compact"} else html.replace("{{name}}", "Ada"), css
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
    return PdfReader(io.BytesIO(pdf))


def children(node):
    kids = node.get("/K", [])
    if not isinstance(kids, list):
        kids = [kids]
    return [kid.get_object() for kid in kids
            if hasattr(kid.get_object(), "get") and "/S" in kid.get_object()]


def structures(reader):
    def walk(node):
        for child in children(node):
            yield child
            yield from walk(child)
    return list(walk(reader.trailer["/Root"]["/StructTreeRoot"]))


@pytest.mark.parametrize("profile", PROFILES)
@pytest.mark.parametrize("mode", MODES)
def test_list_attributes_match_visible_numbering_in_every_render_lane(profile, mode):
    cases = [("ul", "disc", "Disc"), ("ul", "circle", "Circle"),
             ("ul", "square", "Square"), ("ol", "decimal", "Decimal"),
             ("ol", "decimal-leading-zero", "Decimal"),
             ("ol", "lower-roman", "LowerRoman"), ("ol", "upper-roman", "UpperRoman"),
             ("ol", "lower-alpha", "LowerAlpha"), ("ol", "upper-alpha", "UpperAlpha")]
    html = "<h1>Lists for {{name}}</h1>" + "".join(
        f'<{tag} style="list-style-type:{style}"><li>First</li><li>Second</li></{tag}>'
        for tag, style, _ in cases
    )
    reader = render(html, profile, mode)
    repeats = 1 if mode in {"direct", "compiled"} else 2
    lists = [node for node in structures(reader) if node["/S"] == "/L"]
    assert len(lists) == len(cases) * repeats
    assert [node.get("/A", {}).get("/ListNumbering") for node in lists] == [
        "/" + value for _, _, value in cases
    ] * repeats
    assert all(node["/A"]["/O"] == "/List" for node in lists)


@pytest.mark.parametrize("profile", PROFILES)
def test_nested_lists_and_item_overrides_keep_their_own_numbering(profile):
    reader = render('''<ol><li>{{name}}<ul style="list-style-type:square"><li>Child</li></ul></li>
    <li>Second</li></ol><ul><li style="list-style-type:circle">Circle</li></ul>
    <dl><dt>Workshop</dt><dd>Learn together</dd></dl>''', profile)
    lists = [node for node in structures(reader) if node["/S"] == "/L"]
    assert [node.get("/A", {}).get("/ListNumbering") for node in lists] == [
        "/Decimal", "/Square", "/Circle", "/Description" if profile == "pdfua2" else "/None"
    ]
    assert lists[1]["/P"]["/S"] == "/LBody"


@pytest.mark.parametrize("profile", PROFILES)
@pytest.mark.parametrize("mode", MODES)
def test_figure_caption_remains_with_its_image_before_or_after(profile, mode):
    image = f'<img src=\'{IMAGE}\' alt="Green rectangle">'
    html = ("<h1>Figures for {{name}}</h1><p>Before.</p>"
            f'<figure>{image}<figcaption>First caption.</figcaption></figure>'
            '<p>Between.</p>'
            f'<figure><figcaption><p>Second <span>caption.</span></p></figcaption>{image}</figure>'
            '<p>After.</p>')
    reader = render(html, profile, mode)
    repeats = 1 if mode in {"direct", "compiled"} else 2
    captions = [node for node in structures(reader) if node["/S"] == "/Caption"]
    assert len(captions) == 2 * repeats
    for index, caption in enumerate(captions):
        parent = caption["/P"]
        siblings = children(parent)
        assert parent["/S"] == "/Figure", "Preserve the HTML figure's semantic boundary"
        assert parent["/Alt"] == "Green rectangle"
        assert [node["/S"] for node in siblings] == (
            ["/Span", "/Caption"] if index % 2 == 0 else ["/Caption", "/Span"]
        )
        image_node = next(node for node in siblings if node["/S"] == "/Span")
        assert "/Alt" not in image_node, "Do not announce the image description twice"
        assert isinstance(image_node["/K"], int), "Keep the graphic's MCID"
        assert children(caption)[0]["/S"] == "/P"


@pytest.mark.parametrize("display", ["flex", "grid"])
def test_figure_group_survives_css_layout(display):
    html = (f'<figure style="display:{display};flex-direction:column;grid-template-columns:1fr">'
            f'<img src=\'{IMAGE}\' alt="Green rectangle">'
            '<figcaption>A caption.</figcaption></figure>')
    nodes = structures(render(html, "pdfua2"))
    caption = next(node for node in nodes if node["/S"] == "/Caption")
    assert caption["/P"]["/S"] == "/Figure"
    assert caption["/P"]["/Alt"] == "Green rectangle"
    assert [node["/S"] for node in children(caption["/P"])] == ["/Span", "/Caption"]


@pytest.mark.parametrize("profile", PROFILES)
def test_list_numbering_is_retained_when_a_list_splits_across_pages(profile):
    html = '<ol style="list-style-type:upper-alpha">' + ''.join(
        f'<li>Item {n}. A paragraph that occupies enough space to paginate.</li>'
        for n in range(24)
    ) + '</ol>'
    reader = render(html, profile, css=CSS.replace('400pt 600pt', '300pt 180pt'))
    assert len(reader.pages) > 1
    lists = [node for node in structures(reader) if node["/S"] == "/L"]
    assert len(lists) > 1
    assert all(node.get("/A", {}).get("/ListNumbering") == "/UpperAlpha" for node in lists)


@pytest.mark.parametrize("profile", PROFILES)
def test_custom_and_mixed_markers_do_not_claim_a_specific_numbering_style(profile):
    html = """<ul style="list-style-type: '+ '"><li>One</li></ul>
    <ol><li style="list-style-type:upper-roman">One</li><li>Two</li></ol>"""
    lists = [node for node in structures(render(html, profile)) if node["/S"] == "/L"]
    assert [node.get("/A", {}).get("/ListNumbering") for node in lists] == (
        ["/Unordered", "/Ordered"] if profile == "pdfua2" else ["/None", "/None"]
    )


@pytest.mark.parametrize("profile", PROFILES)
def test_multi_image_figures_preserve_each_alternative_and_the_shared_caption(profile):
    html = (f'<figure><img src=\'{IMAGE}\' alt="First rectangle">'
            f'<img src=\'{IMAGE}\' alt="Second rectangle">'
            '<figcaption>Two related illustrations.</figcaption></figure>')
    nodes = structures(render(html, profile))
    caption = next(node for node in nodes if node["/S"] == "/Caption")
    group = caption["/P"]
    assert group["/S"] == "/Sect"
    assert "/Alt" not in group
    assert [node["/S"] for node in children(group)] == ["/Figure", "/Figure", "/Caption"]
    assert [node["/Alt"] for node in children(group) if node["/S"] == "/Figure"] == [
        "First rectangle", "Second rectangle"
    ]


def test_non_graphical_figure_keeps_real_text_and_caption_without_invented_alt():
    reader = render('<figure><p>A quotation, not an image.</p>'
                    '<figcaption>Quotation source.</figcaption></figure>', "pdfua2")
    caption = next(node for node in structures(reader) if node["/S"] == "/Caption")
    assert caption["/P"]["/S"] == "/Sect"
    assert "/Alt" not in caption["/P"]
    assert [node["/S"] for node in children(caption["/P"])] == ["/P", "/Caption"]


def test_missing_image_alt_stays_missing_instead_of_using_its_caption():
    reader = render(f'<figure><img src=\'{IMAGE}\'>'
                    '<figcaption>A caption cannot substitute for missing image semantics.</figcaption></figure>', "pdfua2")
    figures = [node for node in structures(reader) if node["/S"] == "/Figure"]
    assert len(figures) == 1
    assert "/Alt" not in figures[0]
