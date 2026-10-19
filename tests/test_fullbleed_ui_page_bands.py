from dataclasses import FrozenInstanceError, replace
from html.parser import HTMLParser
import io
from pathlib import Path

import pytest
import pypdfium2 as pdfium
from pypdf import PdfReader

from fullbleed.ui.experimental import (
    AuthoringError, Document, Heading, Node, Page, PageBand, PageBandError,
    PageNumbers, Paragraph, Renderer, Style, Text, Theme, component, mm, pt, styled, token,
    css,
)
from fullbleed.ui.experimental._resources import Resource


ROOT = Path(__file__).resolve().parents[1]
THEME = Theme({"accent": "#244c85"}, text=Style(font_family='"Fullbleed Preview Mono"', font_size=pt(10), line_height=pt(12)))


def renderer(**options):
    return Renderer(font_files=[str(ROOT / 'src/preview_fonts/FullbleedPreviewMono-Regular.ttf')],
                    document_timestamp='2026-10-19T00:00:00Z', **options)


def document(*, header=None, footer=None, styles=None):
    return Document(*(Node("section", (Heading(text, level=1 if index == 0 else 2),),
                            style=Style(break_after="page")) for index, text in enumerate(("First", "Second", "Third"))),
                    title="Page bands", theme=THEME, styles=styles,
                    page=Page((pt(320), pt(240)), (pt(40), pt(20), pt(40), pt(20)), header=header, footer=footer))


class Markup(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.tags, self.text = [], []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def handle_data(self, text):
        self.text.append(text)


def test_page_geometry_keeps_positional_contract_and_bands_snapshot_children():
    values = [Text("Brand")]
    calls = []
    def children():
        calls.append(True)
        yield from values
    band = PageBand(left=children(), right=PageNumbers(total=True))
    values.append(Text("Changed"))
    page = Page("LETTER", mm(12), True, header=band)
    assert page.to_css() == "@page { size: LETTER landscape; margin: 12mm; }"
    assert calls == [True] and len(band.left) == 1
    with pytest.raises(FrozenInstanceError):
        band.first_page = False
    with pytest.raises(TypeError):
        Page("A4", mm(10), False, band)
    with pytest.raises(AuthoringError, match="expects a PageBand"):
        Page(header=Text("Not a band"))


def test_page_numbers_are_deferred_slot_values_not_body_nodes():
    with pytest.raises(AuthoringError) as caught:
        Paragraph(PageNumbers())
    assert caught.value.code == "page_numbers.slot"
    with pytest.raises(AuthoringError, match="owns the position"):
        PageNumbers(style=css("position:absolute"))
    with pytest.raises(AuthoringError, match="boolean"):
        PageBand(first_page=1)


def test_html_bands_export_once_outside_main_with_honest_page_placeholders(monkeypatch):
    import fullbleed
    def forbidden(*args, **kwargs):
        raise AssertionError("HTML export constructed a renderer")
    monkeypatch.setattr(fullbleed, "PdfEngine", forbidden)
    doc = document(header=PageBand(left=Text("Brand", class_name="brand")),
                   footer=PageBand(right=PageNumbers(total=True, prefix="Sheet < ", separator=" / ", suffix=" >", class_name="page-count")))
    decorated = doc.to_html(include_styles=True)
    plain = doc.to_html(include_styles=False)
    markup = Markup(plain)
    names = [tag for tag, _ in markup.tags]
    assert names.count('header') == names.count('main') == names.count('footer') == 1
    assert names.index('header') < names.index('main') < names.index('footer')
    assert 'Sheet &lt; ' in plain and '[page]' in plain and '[pages]' in plain and ' &gt;' in plain
    assert 'position: running' not in decorated and 'counter(page)' not in decorated
    assert not any(tag in {'style', 'link'} or 'style' in attrs for tag, attrs in markup.tags)
    assert {attrs['class'] for _, attrs in markup.tags if 'class' in attrs} == {'brand', 'page-count'}
    assert '<style>' in decorated and 'data-fb-page-counter' in decorated


def test_shared_band_components_keep_styles_resources_tokens_and_source_paths():
    resource = Resource("brand", "css", ".brand { letter-spacing: 0.1pt; }")
    Label = styled(Paragraph, color=token('accent'), margin=0)
    @component
    def Brand():
        return replace(Label("Brand", class_name="brand"), _resources=(resource,))
    brand = Brand()
    doc = Document(brand, Heading("Body"), page=Page(header=PageBand(left=brand)), theme=THEME)
    source = doc.compile(target='pdf')
    assert source.css.count(resource.text) == 1
    assert doc.required_tokens == ('accent',)
    assert any('Page.header.left' in item.path and 'Brand' in item.path and item.file for item in source.sources)
    assert any(item.path == ('Brand', 'p') for item in source.sources)
    assert source.html.count('class="brand ') == 2
    assert source == doc.compile(target='pdf')


@pytest.mark.parametrize('css', [
    '@page { @top-left { content:"conflict" } }',
    '@media print { @page :first { @top-left { content:none } } }',
    '@layer override { @page report { @top-left { color:red } } }',
    r'@\70 age { @top\2d left {content:"escaped"} }',
])
def test_band_slot_ownership_uses_native_css_rules(css):
    with pytest.raises(AuthoringError) as caught:
        document(header=PageBand(left="Brand"), styles=css).compile()
    assert caught.value.code == 'styles.page_band' and caught.value.path[-1] == 'top-left'


def test_unowned_slots_and_quoted_fake_rules_remain_available():
    css = '''/* @page { @top-left {content:"comment"} } */
    p::before {content:"@page { @top-left { content:none } }"}
    @page { @bottom-left {content:"Elsewhere"} }'''
    assert 'Elsewhere' in document(header=PageBand(left="Brand"), styles=css).compile().css


def test_component_resources_cannot_override_an_owned_band_slot():
    node = replace(Text('Brand'), _resources=(Resource('conflict', 'css', '@page {@top-left {content:none}}'),))
    with pytest.raises(AuthoringError) as caught:
        document(header=PageBand(left=node)).compile()
    assert caught.value.code == 'styles.page_band'


def test_page_bands_reuse_components_and_finalized_numbers_in_saved_pdf(tmp_path):
    doc = document(header=PageBand(left=Paragraph('BRAND', style=Style(margin=0, color=token('accent')))),
                   footer=PageBand(left='Record', right=PageNumbers(total=True)))
    engine = renderer()
    data = doc.render_pdf(engine, a11y_mode='raise')
    (tmp_path / 'bands.pdf').write_bytes(data)
    texts = [page.extract_text() for page in PdfReader(io.BytesIO(data)).pages]
    assert len(texts) == 3
    for index, text in enumerate(texts, 1):
        assert text.count('BRAND') == 1 and text.count('Record') == 1
        assert f'Page {index} of 3' in text and '[page]' not in text
    plan = doc.compile(target='pdf')
    assert data == bytes(engine.engine_for(doc).render_pdf(plan.html, plan.css))
    assert data == doc.render_pdf(engine)


def test_first_page_suppression_keeps_final_total():
    doc = document(footer=PageBand(right=PageNumbers(total=True), first_page=False))
    texts = [page.extract_text() for page in PdfReader(io.BytesIO(doc.render_pdf(renderer()))).pages]
    assert len(texts) == 3 and 'Page ' not in texts[0]
    assert 'Page 2 of 3' in texts[1] and 'Page 3 of 3' in texts[2]


def test_fit_error_maps_native_source_and_preserves_measurements():
    @component
    def LargeHeader():
        return Paragraph('Too tall', style=Style(height=pt(100), margin=0))
    with pytest.raises(PageBandError) as caught:
        document(header=PageBand(left=LargeHeader())).render_pdf(renderer())
    error = caught.value
    assert error.page == 1 and error.slot == 'top-left' and error.code == 'layout.page_margin_size'
    assert any('LargeHeader' in source.path and source.file for source in error.sources)
    assert error.details['units'] == 'millipoints'
    details = error.details
    details['page'] = 99
    assert error.page == error.details['page'] == 1
    assert error.at('Report').to_dict()['slot'] == 'top-left'


def test_unembedded_font_failure_gives_actionable_authoring_context():
    doc = Document(Heading('Body'), page=Page(header=PageBand(left='Brand')))
    with pytest.raises(PageBandError) as caught:
        doc.render_pdf(Renderer())
    assert caught.value.reason == 'paint_bounds_unknown'
    assert 'Renderer(font_files=' in str(caught.value) and 'Page.header.left' in str(caught.value)


def test_number_class_styles_reach_finalized_text_and_box():
    # Leave breathing room for the conservative native stroke envelope.
    declarations = 'font:italic 700 8pt/10pt "Fullbleed Preview Mono";color:#123456;background:#d9ede3;margin:6pt;padding:2pt 4pt;border:1pt solid #244c85'
    from_class = document(footer=PageBand(right=PageNumbers(total=True, class_name='page-count')),
                          styles='.page-count {' + declarations + '}')
    inline = document(footer=PageBand(right=PageNumbers(total=True, style=css(declarations))))
    data = from_class.render_pdf(renderer())
    assert data == inline.render_pdf(renderer())
    assert data != document(footer=PageBand(right=PageNumbers(total=True))).render_pdf(renderer())
    # Compare actual saved-PDF pixels with an ordinary styled component carrying
    # literal page values. Matching two PageNumbers paths alone could hide a
    # shared failure to carry the style into finalization.
    def pixels(data):
        result = []
        with pdfium.PdfDocument(data) as pdf:
            for page in pdf:
                try:
                    bitmap = page.render(scale=2)
                    try:
                        result.append(bitmap.to_pil().convert('RGB').tobytes())
                    finally:
                        bitmap.close()
                finally:
                    page.close()
        return result
    expected = []
    for index, title in enumerate(('First', 'Second', 'Third'), 1):
        literal = Document(Heading(title, level=1 if index == 1 else 2), title='Page bands', theme=THEME,
                           page=Page((pt(320), pt(240)), (pt(40), pt(20), pt(40), pt(20)),
                                     footer=PageBand(right=Node('div', (f'Page {index} of 3',), style=css(declarations)))))
        expected.extend(pixels(literal.render_pdf(renderer())))
    assert pixels(data) == expected


def test_theme_font_shorthand_reaches_finalized_numbers():
    doc = Document(Heading('Body'), theme=Theme(text=css('font:10pt/12pt "Fullbleed Preview Mono"')),
                   page=Page(footer=PageBand(right=PageNumbers())))
    text = PdfReader(io.BytesIO(doc.render_pdf(renderer()))).pages[0].extract_text()
    assert 'Page 1' in text


def test_number_transforms_are_checked_after_finalization():
    doc = document(footer=PageBand(right=PageNumbers(style=css('transform:translateX(500pt)'))))
    with pytest.raises(PageBandError) as caught:
        doc.render_pdf(renderer())
    assert caught.value.reason == 'paint_overflow' and caught.value.slot == 'bottom-right'


@pytest.mark.parametrize('blank_pages', [False, True])
def test_blank_page_policy_keeps_real_page_values(blank_pages):
    doc = Document(Node('section', (Heading('First'),), style=Style(break_after='right')), Heading('Second', level=2),
                   theme=THEME, page=Page((pt(320), pt(240)), pt(40),
                       footer=PageBand(right=PageNumbers(total=True), blank_pages=blank_pages)))
    texts = [page.extract_text() for page in PdfReader(io.BytesIO(doc.render_pdf(renderer()))).pages]
    assert len(texts) == 3
    assert ('Page 2 of 3' in texts[1]) is blank_pages
    assert 'Page 3 of 3' in texts[2]


@pytest.mark.parametrize('compression', ['throughput', 'compact'])
def test_page_values_finalize_per_reflow_record_with_changing_body_length(compression):
    def report(person, message):
        return Document(Heading('Report'), Paragraph(message), title='Record bands', theme=THEME,
                        page=Page((pt(320), pt(240)), pt(40),
                                  header=PageBand(left=Text(person)),
                                  footer=PageBand(right=PageNumbers(total=True))))

    people = ['Ada', 'Beatrice', 'Ada']
    messages = ['Short record.', 'A longer record with text that flows onto more pages. ' * 32, 'Short record.']
    engine = renderer()
    controls = [report(person, message).render_pdf(engine) for person, message in zip(people, messages)]
    control_text = [[page.extract_text() for page in PdfReader(io.BytesIO(data)).pages] for data in controls]
    assert len(control_text[0]) == 1 and len(control_text[1]) > 1
    for person, pages in zip(people, control_text):
        for index, page in enumerate(pages, 1):
            assert person in page and f'Page {index} of {len(pages)}' in page

    prepared = report('{{person}}', '{{message}}').prepare(engine)
    bindings = {'person': people, 'message': messages}
    data = bytes(prepared.render_pdf_reflow_bindings(bindings, compression=compression))
    actual = [page.extract_text() for page in PdfReader(io.BytesIO(data)).pages]
    assert actual == [page for record in control_text for page in record]
    assert data == bytes(prepared.render_pdf_reflow_bindings(bindings, compression=compression))


def test_generated_numbers_fit_error_identifies_authored_slot():
    doc = document(footer=PageBand(right=PageNumbers(style=Style(height=pt(100)))))
    with pytest.raises(PageBandError) as caught:
        doc.render_pdf(renderer())
    assert caught.value.slot == 'bottom-right' and caught.value.page == 1
    assert any('Page.footer.right' in source.path for source in caught.value.sources)


def test_page_number_labels_are_literals_in_both_export_paths():
    prefix, separator, suffix = 'Sheet "<', ' / ', '>\\end'
    doc = document(footer=PageBand(right=PageNumbers(total=True, prefix=prefix, separator=separator, suffix=suffix)))
    html = doc.to_html(include_styles=False)
    assert 'Sheet &quot;&lt;' in html and '&gt;\\end' in html
    texts = [page.extract_text() for page in PdfReader(io.BytesIO(doc.render_pdf(renderer()))).pages]
    assert all(f'{prefix}{index}{separator}3{suffix}' in text for index, text in enumerate(texts, 1))


def test_compatibility_engine_helper_preserves_page_band_error_context():
    doc = document(header=PageBand(left=Paragraph('Too tall', style=Style(height=pt(100)))))
    with pytest.raises(PageBandError) as caught:
        renderer().checked_engine_for(doc, doc.compile(target='pdf'))
    assert caught.value.slot == 'top-left'


@pytest.mark.parametrize('declaration', ['display:none', 'visibility:hidden'])
def test_hidden_page_number_style_does_not_fall_back_to_visible_margin_text(declaration):
    doc = document(footer=PageBand(right=PageNumbers(class_name='folio')), styles='.folio {' + declaration + '}')
    texts = [page.extract_text() for page in PdfReader(io.BytesIO(doc.render_pdf(renderer()))).pages]
    assert len(texts) == 3 and not any('Page ' in text for text in texts)
