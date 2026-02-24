from __future__ import annotations

import warnings

import pytest

from fullbleed.ui import Document, el, mount_component_html, render_node
from fullbleed.ui.accessibility import (
    A11yContract,
    A11yValidationError,
    A11yWarning,
    FieldGrid,
    FieldItem,
    Figure,
    FigCaption,
    Main,
    Region,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
    SignatureBlock,
    SrText,
    ColumnHeader,
    DataCell,
)


def test_field_grid_emits_dl_dt_dd_semantics() -> None:
    node = FieldGrid(FieldItem("Name", "Jane"), FieldItem("Status", "Signed"))
    html = render_node(node)
    assert html.startswith("<dl ")
    assert 'class="ui-dl ui-field-grid"' in html
    assert 'data-fb-a11y-field-grid="true"' in html
    assert "<dt" in html and "<dd" in html


def test_semantic_table_headers_emit_scope() -> None:
    node = SemanticTable(
        SemanticTableHead(
            SemanticTableRow(
                ColumnHeader("Amount"),
            )
        ),
        SemanticTableBody(
            SemanticTableRow(
                DataCell("$10.00"),
            )
        ),
    )
    html = render_node(node)
    assert "<table" in html
    assert 'scope="col"' in html
    assert "<thead" in html and "<tbody" in html


def test_signature_block_emits_textual_status_and_decorative_mark_by_default() -> None:
    node = SignatureBlock(
        signature_status="captured",
        signer_name="Jane Doe",
        timestamp="2026-02-23T10:30:00Z",
        signature_method="drawn_electronic",
        reference_id="sig-123",
        mark_src="sig.png",
    )
    html = render_node(node)
    assert "Signature status" in html
    assert "Jane Doe" in html
    assert 'src="sig.png"' in html
    assert 'aria-hidden="true"' in html
    assert 'alt=""' in html


def test_signature_mark_svg_defaults_to_signature_label_when_informative() -> None:
    svg = el("svg", viewBox="0 0 10 10")
    node = SignatureBlock(
        signature_status="present",
        signer_name="Jane Doe",
        signature_method="typed_name",
        mark_node=svg,
        mark_decorative=False,
    )
    html = render_node(node)
    assert 'aria-label="Signature: Jane Doe"' in html


def test_a11y_contract_reports_duplicate_ids_and_missing_idrefs() -> None:
    node = Main(
        Region("A", role="region", id="x"),
        Region("B", role="region", id="x"),
        SrText("More", id="help"),
        Region("C", role="region", aria_labelledby="missing"),
    )
    report = A11yContract().validate(node, mode=None)
    codes = {diag["code"] for diag in report["diagnostics"]}
    assert "ID_DUPLICATE" in codes
    assert "IDREF_MISSING" in codes


def test_to_html_raise_mode_fails_on_missing_image_alt() -> None:
    @Document(title="A11y", bootstrap=False)
    def app() -> object:
        return Figure(
            # No alt/aria label and not decorative: should fail strict mode.
            el("img", src="x.png"),
            FigCaption("A figure"),
        )

    artifact = app()
    with pytest.raises(A11yValidationError):
        artifact.to_html(a11y_mode="raise")


def test_mount_component_html_warn_mode_emits_warnings_and_returns_html() -> None:
    def app() -> object:
        return Region("Unlabeled region", role="region")

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        html = mount_component_html(app, a11y_mode="warn")
    assert "<html" in html
    assert any(isinstance(w.message, A11yWarning) for w in caught)
