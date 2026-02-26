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
    RowHeader,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
)
from ..primitives import Box, LayoutGrid, Stack, Text
from ._core import CavKitBase, CavProfile


MARRIAGE_RECORD_FAMILY_ID = "marriage_record_cav"


FL_ESCAMBIA_MARRIAGE_RECORD_REV2019 = CavProfile(
    profile_id="fl.escambia.marriage_record.rev2019",
    profile_version=1,
    family_id=MARRIAGE_RECORD_FAMILY_ID,
    revision="rev2019",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Florida Department of Health / Office of Vital Statistics",
    display_name="Florida Escambia Marriage Record (Rev 2019)",
    coverage_notes=(
        "Profile scope is county/revision specific.",
        "Claims attach to the profile, not the family kit broadly.",
    ),
)

FL_ESCAMBIA_MARRIAGE_RECORD_REV2016 = CavProfile(
    profile_id="fl.escambia.marriage_record.rev2016",
    profile_version=1,
    family_id=MARRIAGE_RECORD_FAMILY_ID,
    revision="rev2016",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Florida Department of Health / Office of Vital Statistics",
    display_name="Florida Escambia Marriage Record (Rev 2016)",
    coverage_notes=("Layout/revision scope differs from rev2019.",),
)


@dataclass
class MarriageRecordCavKit(CavKitBase):
    family_id: str = MARRIAGE_RECORD_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "recording_annotation",
        "header",
        "record_header",
        "application_to_marry",
        "license_to_marry",
        "certificate_of_marriage",
        "footer_notice",
        "signatures",
        "source_pdf",
        "source_pdf_path",
        "source_analysis",
        "review_queue",
        "metadata",
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
        data = dict(payload)
        return _marriage_record_document(data)


def _sig_name(sig: Mapping[str, Any]) -> str:
    signer = str(sig.get("signer_name") or "").strip()
    return signer or "[Illegible in source scan]"


def _sig_date(sig: Mapping[str, Any]) -> str:
    value = str(sig.get("signed_on") or "").strip()
    return value or "[Date not recorded]"


def _sig_status(sig: Mapping[str, Any]) -> str:
    value = str(sig.get("signature_status") or "unknown").strip().replace("_", " ")
    return value or "unknown"


def _sig_method(sig: Mapping[str, Any]) -> str:
    value = str(sig.get("signature_method") or "unknown").strip().replace("_", " ")
    return value or "unknown"


def _signature_table(signatures: list[Mapping[str, Any]], *, title: str, table_id: str) -> object:
    rows: list[Any] = []
    for item in signatures:
        status_text = el(
            "span",
            _sig_status(item),
            data_fb_a11y_signature_status=str(item.get("signature_status") or "unknown"),
            data_fb_a11y_signature_method=str(item.get("signature_method") or "unknown"),
            data_fb_a11y_signature_ref=str(item.get("reference_id") or ""),
        )
        rows.append(
            SemanticTableRow(
                RowHeader(str(item.get("role") or "Signature record")),
                DataCell(status_text),
                DataCell(_sig_name(item)),
                DataCell(_sig_date(item)),
                DataCell(_sig_method(item)),
            )
        )
    return SemanticTable(
        SemanticTableHead(
            SemanticTableRow(
                ColumnHeader("Role", class_name="role-col"),
                ColumnHeader("Status", class_name="status-col"),
                ColumnHeader("Signer", class_name="signer-col"),
                ColumnHeader("Date", class_name="date-col"),
                ColumnHeader("Method", class_name="method-col"),
            )
        ),
        SemanticTableBody(*rows),
        caption=title,
        id=table_id,
        class_name="sig-table",
    )


def _person_card(person: Mapping[str, Any], *, heading_id: str) -> object:
    return Box(
        Heading(str(person.get("label") or "Applicant"), level=3, id=heading_id),
        FieldGrid(
            FieldItem("Name", person.get("full_name") or "[Not legible]"),
            FieldItem("Maiden surname", person.get("maiden_surname") or "[Blank on form]"),
            FieldItem("Date of birth", person.get("date_of_birth") or "[Not legible]"),
            FieldItem("Residence city", person.get("residence_city") or "[Not legible]"),
            FieldItem("County", person.get("county") or "[Not legible]"),
            FieldItem("State", person.get("state") or "[Not legible]"),
            FieldItem("Birthplace", person.get("birthplace") or "[Not legible]"),
        ),
        class_name="grid-card",
        role="group",
        aria_labelledby=heading_id,
    )


@Document(
    page="LETTER",
    margin="0.35in",
    title="Marriage Record CAV (Reauthored Golden)",
    bootstrap=False,
    lang="en-US",
)
def _marriage_record_document(payload: dict[str, Any]) -> object:
    record_header = payload["record_header"]
    app = payload["application_to_marry"]
    lic = payload["license_to_marry"]
    cert = payload["certificate_of_marriage"]

    return Stack(
        Region(
            Text(record_header["recorded_stamp_text"], tag="p", class_name="stamp-line"),
            label="Recorded filing annotation",
            class_name="a11y-section",
        ),
        Box(
            Heading("STATE OF FLORIDA MARRIAGE RECORD", level=1),
            Text(
                "Compliant Alternative Version (CAV) preserving the document information from the scanned source.",
                tag="p",
                class_name="subtitle",
            ),
            class_name="doc-title",
        ),
        Section(
            Heading("Record Header", level=2, id="record-header-heading"),
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem("Agency", record_header["agency"]),
                        FieldItem("Jurisdiction", record_header["jurisdiction"]),
                        FieldItem("Form title", record_header["form_title"]),
                        FieldItem("Application number", record_header["application_number"]),
                        FieldItem(
                            "State file number",
                            record_header.get("state_file_number") or "[Blank on source form]",
                        ),
                    ),
                    class_name="grid-card",
                ),
                class_name="meta-grid",
            ),
            class_name="a11y-section",
        ),
        Section(
            Heading("APPLICATION TO MARRY", level=2, id="application-heading"),
            LayoutGrid(
                _person_card(app["applicant_1"], heading_id="applicant-1-heading"),
                _person_card(app["applicant_2"], heading_id="applicant-2-heading"),
                class_name="person-grid",
            ),
            Text(
                "WE THE APPLICANTS NAMED IN THIS CERTIFICATE, EACH FOR HIMSELF OR HERSELF, STATE THAT THE INFORMATION PROVIDED ON THIS RECORD IS CORRECT TO THE BEST OF OUR KNOWLEDGE AND BELIEF, THAT NO LEGAL OBJECTION TO THE MARRIAGE NOR THE ISSUANCE OF A LICENSE TO AUTHORIZE THE SAME IS KNOWN TO US AND HEREBY APPLY FOR LICENSE TO MARRY.",
                tag="p",
                class_name="boilerplate",
            ),
            _signature_table(
                list(app["signatures"]),
                title="Application signatures and notarization records",
                table_id="application-signatures-table",
            ),
            Text("SEAL PRESENT (application notarization)", tag="p", class_name="seal-note"),
            class_name="a11y-section",
        ),
        Section(
            Heading("LICENSE TO MARRY", level=2, id="license-heading"),
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem("County issuing license", lic["county_issuing_license"]),
                        FieldItem("Date license issued", lic["date_license_issued"]),
                        FieldItem("Date license effective", lic["date_license_effective"]),
                        FieldItem("Expiration date", lic["expiration_date"]),
                        FieldItem("Clerk title", lic["clerk_title"]),
                        FieldItem("By D.C. initials", lic["by_dc_initials"] or "[Illegible in source scan]"),
                    ),
                    class_name="grid-card",
                ),
                class_name="license-grid",
            ),
            Text(
                "AUTHORIZATION AND LICENSE IS HEREBY GIVEN TO ANY PERSON DULY AUTHORIZED BY THE LAWS OF THE STATE OF FLORIDA TO PERFORM A MARRIAGE CEREMONY WITHIN THE STATE OF FLORIDA AND TO SOLEMNIZE THE MARRIAGE OF THE ABOVE NAMED PERSONS. THIS LICENSE MUST BE USED ON OR AFTER THE EFFECTIVE DATE AND ON OR BEFORE THE EXPIRATION DATE IN THE STATE OF FLORIDA IN ORDER TO BE RECORDED AND VALID.",
                tag="p",
                class_name="boilerplate",
            ),
            _signature_table(
                list(lic["signatures"]),
                title="License signatures and clerk markings",
                table_id="license-signatures-table",
            ),
            Text("SEAL PRESENT (license issuing seal)", tag="p", class_name="seal-note"),
            class_name="a11y-section",
        ),
        Section(
            Heading("CERTIFICATE OF MARRIAGE", level=2, id="certificate-heading"),
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem("Date of marriage", cert["date_of_marriage"]),
                        FieldItem("Location of marriage", cert["location_of_marriage"]),
                        FieldItem("Performer address", cert["performer_address"]),
                        FieldItem(
                            "Performer name and title",
                            cert["performer_name_title_transcription"],
                        ),
                    ),
                    class_name="grid-card",
                ),
                class_name="certificate-grid",
            ),
            Text(
                "I HEREBY CERTIFY THAT THE ABOVE NAMED SPOUSES WERE JOINED BY ME IN MARRIAGE IN ACCORDANCE WITH THE LAWS OF THE STATE OF FLORIDA.",
                tag="p",
                class_name="boilerplate",
            ),
            _signature_table(
                list(cert["signatures"]),
                title="Certificate signatures",
                table_id="certificate-signatures-table",
            ),
            class_name="a11y-section",
        ),
        Text(
            "INFORMATION BELOW FOR USE BY VITAL STATISTICS ONLY - NOT TO BE RECORDED",
            tag="p",
            class_name="final-line",
        ),
        class_name="cav2-root",
    )
