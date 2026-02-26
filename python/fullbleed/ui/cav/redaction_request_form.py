# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping

from ..core import Document, el
from ..accessibility import (
    ColumnHeader,
    DataCell,
    FieldGrid,
    FieldItem,
    Heading,
    Region,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
    SignatureBlock,
)
from ..primitives import Box, LayoutGrid, Stack, Text
from ._core import CavKitBase, CavProfile


REQUEST_REDACTION_FORM_FAMILY_ID = "request_redaction_form_cav"


FL_ESCAMBIA_REQUEST_REDACTION_EXEMPT_PERSONAL_INFORMATION_EFFECTIVE_2025_V1 = CavProfile(
    profile_id="fl.escambia.request_redaction_exempt_personal_information.effective_2025.v1",
    profile_version=1,
    family_id=REQUEST_REDACTION_FORM_FAMILY_ID,
    revision="effective_2025_03_19_v1",
    jurisdiction="US",
    county="Escambia",
    issuing_authority="Escambia Clerk of the Circuit Court and Comptroller",
    display_name="Request for Redaction of Exempt Personal Information (Effective March 19, 2025) V1",
    supported_variants=(
        "three-page request-for-redaction forms with statutory category checklists, requestor/contact fields, and notary block",
    ),
    coverage_notes=(
        "v1 preserves same-use structure, section headings, blank request lines, release notices, and signature/notary semantics.",
    ),
    unsupported_features=(
        "dynamic e-filing workflow integration and clerk-side adjudication state transitions",
    ),
)


def _list_section(items: list[str], *, class_name: str) -> object:
    children = [el("li", item) for item in items]
    return el("ul", children, class_name=class_name)


def _category_card(title: str, items: list[str], *, class_name: str) -> object:
    return Box(
        Heading(title, level=3),
        _list_section(items, class_name="rrf-list"),
        class_name=class_name,
    )


def _docs_table(rows: list[Mapping[str, Any]]) -> object:
    body_rows = [
        SemanticTableRow(
            DataCell(str(r.get("instrument_number") or "[Blank]")),
            DataCell(str(r.get("book") or "[Blank]")),
            DataCell(str(r.get("page") or "[Blank]")),
            DataCell(str(r.get("document_title") or "[Blank]")),
        )
        for r in rows
    ]
    if not body_rows:
        body_rows = [
            SemanticTableRow(
                DataCell("[Blank]"),
                DataCell("[Blank]"),
                DataCell("[Blank]"),
                DataCell("[Blank]"),
            )
        ]
    return SemanticTable(
        SemanticTableHead(
            SemanticTableRow(
                ColumnHeader("Instrument Number"),
                ColumnHeader("Book"),
                ColumnHeader("Page"),
                ColumnHeader("Document Title"),
            )
        ),
        SemanticTableBody(*body_rows),
        caption="Documents to be redacted",
        class_name="rrf-table",
    )


@dataclass
class RequestRedactionFormCavKit(CavKitBase):
    family_id: str = REQUEST_REDACTION_FORM_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "title_lines",
        "intro_paragraphs",
        "statutory_categories_left",
        "statutory_categories_right",
        "category_note",
        "requestor_contact",
        "information_to_be_redacted",
        "warning_paragraphs",
        "documents_intro_paragraphs",
        "documents_table_rows",
        "documents_other_line",
        "release_to_government_paragraphs",
        "release_for_title_searches_paragraphs",
        "courtesy_notice_paragraphs",
        "notary_block",
        "signature_block",
        "review_queue",
        "metadata",
        "source_pdf",
        "source_pdf_path",
        "source_analysis",
    )

    def render(
        self,
        *,
        payload: Mapping[str, Any],
        claim_evidence: Mapping[str, Any] | None = None,
    ) -> Any:
        scope_report = self.validate_payload_scope(payload)
        if self.strict_scope and not scope_report.get("ok", False):
            raise ValueError(
                f"payload is out of profile scope for {self.profile.profile_id}: {scope_report.get('issues') or []}"
            )
        return _request_redaction_form_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.34in",
    title="Request Redaction Form CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _request_redaction_form_document(payload: dict[str, Any]) -> object:
    contact = dict(payload.get("requestor_contact") or {})
    redact = dict(payload.get("information_to_be_redacted") or {})
    notary = dict(payload.get("notary_block") or {})
    signature = dict(payload.get("signature_block") or {})
    doc_rows = [dict(r) for r in (payload.get("documents_table_rows") or [])]

    page_one = Region(
        Box(
            *[Heading(str(line), level=(1 if i == 0 else 2)) for i, line in enumerate(payload.get("title_lines") or [])],
            class_name="rrf-title-box",
        ),
        Section(
            *[Text(str(p), tag="p", class_name="rrf-body") for p in (payload.get("intro_paragraphs") or [])],
            class_name="rrf-section",
        ),
        Section(
            Heading("Statutory Basis for Removal", level=2),
            LayoutGrid(
                _category_card(
                    "Category Group A",
                    [str(x) for x in (payload.get("statutory_categories_left") or [])],
                    class_name="rrf-card",
                ),
                _category_card(
                    "Category Group B",
                    [str(x) for x in (payload.get("statutory_categories_right") or [])],
                    class_name="rrf-card",
                ),
                class_name="rrf-categories-grid",
            ),
            Text(str(payload.get("category_note") or ""), tag="p", class_name="rrf-note"),
            class_name="rrf-section",
        ),
        label="Request for redaction page 1",
        class_name="rrf-page rrf-page-1",
    )

    page_two = Region(
        Section(
            Heading("Requestor Contact Information", level=2),
            FieldGrid(
                FieldItem("Printed Name", contact.get("printed_name") or "[Blank on form]"),
                FieldItem("Telephone Number", contact.get("telephone") or "[Blank on form]"),
                FieldItem("Email Address", contact.get("email") or "[Blank on form]"),
            ),
            class_name="rrf-section",
        ),
        Section(
            Heading("Information to Be Redacted", level=2),
            FieldGrid(
                FieldItem("Address where I reside", redact.get("residence_address") or "[Blank on form]"),
                FieldItem(
                    "Additional address/description fields",
                    redact.get("additional_address_descriptions") or "[Blank on form]",
                ),
                FieldItem("Telephone Number(s)", redact.get("telephone_numbers") or "[Blank on form]"),
                FieldItem("Social Security Number / Date of Birth", redact.get("ssn_dob") or "[Blank on form]"),
                FieldItem("Name of spouse and/or children", redact.get("spouse_children_names") or "[Blank on form]"),
                FieldItem("Place(s) of employment/location", redact.get("employment_location") or "[Blank on form]"),
                FieldItem(
                    "School/Daycare facility location of child",
                    redact.get("school_daycare_location") or "[Blank on form]",
                ),
                FieldItem("Personal assets", redact.get("personal_assets") or "[Blank on form]"),
            ),
            class_name="rrf-section",
        ),
        Section(
            Heading("Warnings and Public Record Notice", level=2),
            *[Text(str(p), tag="p", class_name="rrf-body") for p in (payload.get("warning_paragraphs") or [])],
            class_name="rrf-section",
        ),
        label="Request for redaction page 2",
        class_name="rrf-page rrf-page-2 page-break-before",
    )

    page_three = Region(
        Section(
            Heading("Documents to Be Redacted", level=2),
            *[Text(str(p), tag="p", class_name="rrf-body") for p in (payload.get("documents_intro_paragraphs") or [])],
            _docs_table(doc_rows),
            FieldGrid(
                FieldItem(
                    "Documents Other Than Official Records",
                    payload.get("documents_other_line") or "[Blank on form]",
                )
            ),
            class_name="rrf-section",
        ),
        Section(
            Heading("Release and Courtesy Notices", level=2),
            *[
                Text(str(p), tag="p", class_name="rrf-body")
                for p in (payload.get("release_to_government_paragraphs") or [])
            ],
            *[
                Text(str(p), tag="p", class_name="rrf-body")
                for p in (payload.get("release_for_title_searches_paragraphs") or [])
            ],
            Box(
                Heading("Courtesy Notice - Release of Prior Redactions", level=3),
                *[Text(str(p), tag="p", class_name="rrf-body") for p in (payload.get("courtesy_notice_paragraphs") or [])],
                class_name="rrf-notice-box",
            ),
            class_name="rrf-section",
        ),
        Section(
            Heading("Signature and Notary", level=2),
            FieldGrid(
                FieldItem("State", notary.get("state") or "FLORIDA"),
                FieldItem("County", notary.get("county") or "[Blank on form]"),
                FieldItem("Sworn statement", notary.get("sworn_statement") or "[Blank on form]"),
                FieldItem("Identity line", notary.get("identity_line") or "[Blank on form]"),
                FieldItem("Notary signature line", notary.get("notary_signature_line") or "[Blank on form]"),
                FieldItem("Notary print/type/stamp", notary.get("notary_print_name") or "[Blank on form]"),
            ),
            SignatureBlock(
                signature_status=str(signature.get("signature_status") or "missing"),
                signer_name=str(signature.get("signer_name") or "Requestor"),
                timestamp=str(signature.get("timestamp")) if signature.get("timestamp") else None,
                signature_method=str(signature.get("signature_method") or "signature_line_only"),
                reference_id=str(signature.get("reference_id") or "requestor-signature"),
                mark_decorative=True,
                class_name="rrf-signature-block",
            ),
            class_name="rrf-section",
        ),
        label="Request for redaction page 3",
        class_name="rrf-page rrf-page-3 page-break-before",
    )

    return Stack(page_one, page_two, page_three, class_name="rrf-root")


__all__ = [
    "REQUEST_REDACTION_FORM_FAMILY_ID",
    "FL_ESCAMBIA_REQUEST_REDACTION_EXEMPT_PERSONAL_INFORMATION_EFFECTIVE_2025_V1",
    "RequestRedactionFormCavKit",
]
