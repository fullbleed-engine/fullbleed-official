"""The intended public import path composes with both existing authoring APIs."""
from fullbleed import components as public
from fullbleed.components import bootstrap
from fullbleed.ui import Document as LegacyDocument, Paragraph as LegacyParagraph
from fullbleed.ui import experimental
from fullbleed.ui.experimental import bootstrap as preview_bootstrap


def test_public_and_preview_components_mix_without_conversion():
    @public.component
    def Notice(message: str):
        return experimental.Paragraph(message, style=public.style(color=public.token("accent")))

    page = experimental.Page("A5", margin=public.mm(12))
    content = public.Flow(Notice("Paid & received"), experimental.Paragraph("Thank you"))
    theme = public.Theme({"accent": "#173b35"})
    current = public.Document(content, title="Receipt", page=page, theme=theme)
    previous = experimental.Document(content, title="Receipt", page=page, theme=theme)
    assert current.compile() == previous.compile()
    assert current.to_html(include_styles=True) == previous.to_html(include_styles=True)
    assert current.to_html(include_styles=False) == previous.to_html(include_styles=False)
    assert "Paid &amp; received" in current.to_html()
    assert public.AuthoringError is experimental.AuthoringError
    assert public.LayoutError is experimental.LayoutError


def test_public_bootstrap_shares_styles_and_types_with_preview_kit():
    column = public.Column("Item", lambda row: row["item"], width=public.pct(100))
    content = public.Flow(
        bootstrap.Alert("Ready", variant="success"),
        preview_bootstrap.Badge("Approved", variant="success"),
        bootstrap.DataTable([{"item": "Document"}], columns=[column]),
    )
    current = public.Document(content)
    previous = experimental.Document(content)
    assert current.compile() == previous.compile()
    assert bootstrap.stylesheet() == preview_bootstrap.stylesheet()
    assert bootstrap.Button is preview_bootstrap.Button
    assert bootstrap.Column is public.Column


def test_legacy_document_decorator_and_explicit_bridge_remain_available():
    @LegacyDocument(title="Existing report", bootstrap=False)
    def Report():
        return [LegacyParagraph("Existing content"), public.to_legacy(public.Paragraph("New content"))]

    artifact = Report()
    assert artifact.title == "Existing report"
    assert "Existing content" in artifact.to_html() and "New content" in artifact.to_html()
    assert LegacyDocument is not public.Document


def test_runtime_exports_preserve_the_preview_contract():
    for facade, preview in ((public, experimental), (bootstrap, preview_bootstrap)):
        assert facade.__all__ == preview.__all__
        assert facade.__all__ is not preview.__all__
        assert all(getattr(facade, name) is getattr(preview, name) for name in facade.__all__)
