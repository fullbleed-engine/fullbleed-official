# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import (
    ColumnHeader,
    DataCell,
    FieldGrid,
    FieldItem,
    Heading,
    Region,
    RowHeader,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
)
from ..primitives import Box, LayoutGrid, Stack, Text

from ._core import CavKitBase, CavProfile


WARRANTY_DEED_FAMILY_ID = "warranty_deed_cav"


FL_ESCAMBIA_WARRANTY_DEED_REV1994 = CavProfile(
    profile_id="fl.escambia.warranty_deed.rev1994",
    profile_version=1,
    family_id=WARRANTY_DEED_FAMILY_ID,
    revision="rev1994",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Clerk / Recorder",
    display_name="Florida Escambia Warranty Deed (Recorded Instrument Rev 1994)",
    coverage_notes=(
        "Profile reflects the recorded instrument layout/revision represented by the Thunderbird sample.",
    ),
)


@dataclass
class WarrantyDeedCavKit(CavKitBase):
    family_id: str = WARRANTY_DEED_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "title",
        "page_count",
        "header",
        "warranty_deed",
        "recorder_markings",
        "witness_and_grantor_signatures",
        "signatures",
        "notary_acknowledgment",
        "schedule_a",
        "page3",
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
            issues = list(scope_report.get("issues") or [])
            raise ValueError(
                f"payload is out of profile scope for {self.profile.profile_id}: {issues}"
            )
        return _warranty_deed_document(dict(payload))


def _sig_span(record: Mapping[str, Any]) -> object:
    return el(
        "span",
        str(record.get("signature_text") or ""),
        class_name="sig-lex",
        data_fb_a11y_signature_status=str(record.get("signature_status") or "unknown"),
        data_fb_a11y_signature_method=str(record.get("signature_method") or "unknown"),
        data_fb_a11y_signature_ref=str(record.get("reference_id") or ""),
    )


def _tabular_rows(items: Iterable[Mapping[str, Any]], *, witness: bool) -> list[Any]:
    rows: list[Any] = []
    for item in items:
        label_col = (
            str(item.get("witness_name") or "[Unknown]")
            if witness
            else str(item.get("printed_name") or "[Unknown]")
        )
        rows.append(
            SemanticTableRow(
                RowHeader(str(item.get("slot") or ("Witness" if witness else "Grantor"))),
                DataCell(_sig_span(item)),
                DataCell(label_col),
                DataCell(str(item.get("name_address_line") or "[Blank line]")),
            )
        )
    return rows


def _signature_table(*, title: str, table_id: str, rows: list[Any], row_label: str) -> object:
    return SemanticTable(
        SemanticTableHead(
            SemanticTableRow(
                ColumnHeader(row_label, class_name="slot-col"),
                ColumnHeader("Signature semantics", class_name="sig-col"),
                ColumnHeader("Printed name", class_name="name-col"),
                ColumnHeader("Name/address line", class_name="addr-col"),
            )
        ),
        SemanticTableBody(*rows),
        caption=title,
        id=table_id,
        class_name="tb-table sig-table",
    )


def _build_page_one(payload: dict[str, Any]) -> object:
    header = payload["header"]
    deed = payload["warranty_deed"]
    recorder = payload["recorder_markings"]["page1_box"]
    sigs = payload["witness_and_grantor_signatures"]
    notary = payload["notary_acknowledgment"]

    deed_paragraphs: list[Any] = [
        Text(p, tag="p", class_name="tb-para") for p in (deed.get("grant_text_paragraphs") or [])
    ]
    deed_paragraphs.append(
        Text(
            f"Parcel Identification Number: {deed.get('parcel_identification_number') or '[Not legible]'}",
            tag="p",
            class_name="tb-para tb-strong",
        )
    )
    deed_paragraphs.extend(
        Text(p, tag="p", class_name="tb-para")
        for p in (deed.get("habendum_and_warranty_paragraphs") or [])
    )

    witness_table = _signature_table(
        title="Witness signature lines",
        table_id="tb-witness-signatures",
        rows=_tabular_rows(list(sigs.get("witness_rows") or []), witness=True),
        row_label="Witness line",
    )
    grantor_table = _signature_table(
        title="Grantor signature lines",
        table_id="tb-grantor-signatures",
        rows=_tabular_rows(list(sigs.get("grantor_rows") or []), witness=False),
        row_label="Grantor line",
    )

    notary_sig = el(
        "span",
        str(notary.get("notary_signature_text") or ""),
        class_name="sig-lex",
        data_fb_a11y_signature_status=str(notary.get("notary_signature_status") or "unknown"),
        data_fb_a11y_signature_method=str(notary.get("notary_signature_method") or "unknown"),
        data_fb_a11y_signature_ref=str(notary.get("notary_signature_ref") or "notary-signature"),
    )

    return Region(
        Box(
            LayoutGrid(
                Box(
                    Heading(str(payload.get("title") or "This Warranty Deed"), level=1),
                    FieldGrid(
                        FieldItem("Execution date text", deed["execution_date_text"]),
                        FieldItem("Grantor(s)", deed["grantors"]),
                        FieldItem("Grantee(s)", deed["grantees"]),
                        FieldItem(
                            "Property address",
                            ", ".join(
                                str(x)
                                for x in (deed.get("property_address_lines") or [])
                                if str(x).strip()
                            ),
                        ),
                        FieldItem("Margin notation", header.get("margin_marking") or "[No notation]"),
                    ),
                    class_name="tb-card tb-title-card",
                ),
                Box(
                    FieldGrid(
                        FieldItem("Book/Page", header["instrument_book_page"]),
                        FieldItem("Instrument number", header["instrument_number"]),
                        FieldItem("D.S. PD.", recorder.get("dsp_d") or "[Illegible]"),
                        FieldItem("Date stamp", recorder.get("date_stamp") or "[Illegible]"),
                        FieldItem("Clerk", recorder.get("clerk_name") or "[Illegible]"),
                        FieldItem("Role", recorder.get("clerk_role") or "[Illegible]"),
                        FieldItem("By", recorder.get("by_line") or "[Illegible in source scan]"),
                        FieldItem("Cert. Reg.", recorder.get("cert_reg") or "[Illegible]"),
                    ),
                    class_name="tb-card tb-recorder-card",
                ),
                class_name="tb-top-grid",
            ),
            class_name="tb-section-box",
        ),
        Box(*deed_paragraphs, class_name="tb-card tb-legal-card"),
        Box(
            Text(str(sigs.get("presence_statement") or ""), tag="p", class_name="tb-para tb-strong"),
            LayoutGrid(
                Box(witness_table, class_name="tb-card"),
                Box(grantor_table, class_name="tb-card"),
                class_name="tb-signature-grid",
            ),
            class_name="tb-section-box",
        ),
        Box(
            FieldGrid(
                FieldItem("State", notary.get("state") or "[Not legible]"),
                FieldItem("County", notary.get("county") or "[Not legible]"),
                FieldItem("Acknowledgment date", notary.get("ack_date_text") or "[Not legible]"),
                FieldItem("Acknowledging parties", notary.get("acknowledging_parties") or "[Not legible]"),
                FieldItem("Identity clause", notary.get("identity_clause") or "[Not legible]"),
                FieldItem("Notary signature semantics", notary_sig),
                FieldItem("Print name line", notary.get("print_name_line") or "[Illegible in source scan]"),
                FieldItem(
                    "Commission expires line",
                    notary.get("commission_expires_line") or "[Illegible in source scan]",
                ),
                FieldItem(
                    "Official seal",
                    "SEAL PRESENT: "
                    + str(notary.get("official_seal_text_visible") or "[Illegible seal text]")
                    if notary.get("official_seal_present")
                    else "No seal visible",
                ),
            ),
            class_name="tb-card tb-notary-card",
        ),
        Box(
            FieldGrid(
                FieldItem("Prepared by / return to", "\n".join(header.get("prepared_by_lines") or [])),
                FieldItem("File number", header.get("file_number") or "[Not visible]"),
            ),
            class_name="tb-card tb-prepared-card",
        ),
        label="Source page 1: Warranty deed body, signatures, and recorder/notary markings",
        class_name="tb-page tb-page-1",
    )


def _build_page_two(payload: dict[str, Any]) -> object:
    schedule = payload["schedule_a"]
    return Section(
        Heading(str(schedule.get("title") or "Schedule A"), level=2),
        LayoutGrid(
            Box(
                FieldGrid(
                    FieldItem(
                        "Instrument reference header",
                        schedule.get("instrument_ref_header") or "[Not legible]",
                    )
                ),
                *[
                    Text(line, tag="p", class_name="tb-legal-description")
                    for line in str(schedule.get("legal_description_text") or "").splitlines()
                    if line.strip()
                ],
                class_name="tb-card tb-schedule-card",
            ),
            Box(
                Heading("Instrument filing stamp", level=3),
                FieldGrid(
                    *[
                        FieldItem(f"Stamp line {idx}", line)
                        for idx, line in enumerate(schedule.get("stamp_lines") or [], start=1)
                    ]
                ),
                class_name="tb-card tb-stamp-card",
            ),
            class_name="tb-schedule-grid",
        ),
        el("pre", " \n" * 34, class_name="tb-page2-spacer", aria_hidden="true"),
        class_name="tb-page tb-page-2 page-break-before",
    )


def _build_page_three(payload: dict[str, Any]) -> object:
    page3 = payload["page3"]
    children: list[Any] = [
        Text(str(page3.get("file_reference") or "[No file reference visible]"), tag="p", class_name="tb-file-ref")
    ]
    if page3.get("horizontal_line_present"):
        children.append(el("hr", class_name="tb-rule"))
    children.append(Text(str(page3.get("body_content_note") or ""), tag="p", class_name="tb-page-note"))
    return Region(
        Box(*children, class_name="tb-card tb-page3-card"),
        label="Source page 3: file reference and linework",
        class_name="tb-page tb-page-3 page-break-before",
    )


@Document(
    page="LETTER",
    margin="0.22in",
    title="Thunderbird Warranty Deed CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _warranty_deed_document(payload: dict[str, Any]) -> object:
    return Stack(
        _build_page_one(payload),
        _build_page_two(payload),
        _build_page_three(payload),
        class_name="tb-root",
    )
