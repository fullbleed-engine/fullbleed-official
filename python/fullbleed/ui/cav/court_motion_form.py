# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import FieldGrid, FieldItem, FieldSet, Heading, Legend, Region, Section
from ..primitives import Box, LayoutGrid, Stack, Text
from ._core import CavKitBase, CavProfile


COURT_MOTION_FORM_FAMILY_ID = "court_motion_form_cav"


FL_ESCAMBIA_COURT_MOTION_FORM_CHILD_SUPPORT_TELEPHONE_HEARING_TITLE_IV_D_2019_V1 = CavProfile(
    profile_id="fl.escambia.court_motion_form.child_support_telephone_hearing_title_iv_d_2019.v1",
    profile_version=1,
    family_id=COURT_MOTION_FORM_FAMILY_ID,
    revision="child_support_telephone_hearing_title_iv_d_2019_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Family Law Division / Child Support Hearing Officer",
    display_name="Escambia Court Motion Form - Child Support Telephone Hearing (Title IV-D) 2019 V1",
    supported_variants=(
        "single-page child support motion forms with court caption, case metadata, grounds checklist, service certification, and signature/contact lines",
    ),
    coverage_notes=(
        "Profile is scoped to the Escambia child support hearing officer motion for authority to participate/testify by telephone (updated 6/2019).",
        "Form lines are represented as accessible field/value rows and checklist text with explicit blank placeholders when blank on source.",
    ),
    unsupported_features=(
        "fillable AcroForm field export in v1 (CAV is delivered as structured HTML/CSS/PDF output)",
    ),
)

FL_ESCAMBIA_COURT_MOTION_FORM_CLERK_DISCHARGE_FORFEITURE_FS903_26_8_2020_V1 = CavProfile(
    profile_id="fl.escambia.court_motion_form.clerk_discharge_forfeiture_fs903_26_8_2020.v1",
    profile_version=1,
    family_id=COURT_MOTION_FORM_FAMILY_ID,
    revision="clerk_discharge_forfeiture_fs903_26_8_2020_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Clerk of the Circuit Court and Comptroller",
    display_name="Escambia Court Motion Form - Application for Clerk's Discharge of Forfeiture (F.S. 903.26(8)) 2020 V1",
    supported_variants=(
        "two-page forfeiture discharge application forms with court caption, bond line items, statutory basis checklist, and signature/contact block",
    ),
    coverage_notes=(
        "Profile is scoped to the Escambia clerk application for discharge of forfeiture revised 07/27/2020 (DocCenter 728).",
        "The CAV preserves blank form lines, checklist rows, and signature/contact semantics with explicit page-group break before statutory/declaration sections.",
    ),
    unsupported_features=(
        "fillable AcroForm field export in v1 (CAV output remains structured HTML/CSS/PDF artifacts)",
    ),
)


def _paragraphs(items: Iterable[str], *, class_name: str = "cmf-body") -> list[Any]:
    return [Text(str(p), tag="p", class_name=class_name) for p in items if str(p).strip()]


def _checkbox_rows(items: Iterable[Mapping[str, Any]]) -> list[Any]:
    rows: list[Any] = []
    for item in items:
        mark = "[x]" if bool(item.get("checked")) else "[ ]"
        text = str(item.get("text") or "[Ground not transcribed]")
        rows.append(el("li", f"{mark} {text}", class_name="cmf-checkbox-item"))
    return rows


@dataclass
class CourtMotionFormCavKit(CavKitBase):
    family_id: str = COURT_MOTION_FORM_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "header_note",
        "court_caption",
        "motion_title_lines",
        "opening_statement",
        "grounds_intro",
        "grounds",
        "warning_paragraphs",
        "service_certification_paragraphs",
        "court_caption_heading",
        "bond_rows_heading",
        "bond_rows",
        "grounds_heading",
        "warning_heading",
        "service_certification_heading",
        "signature_heading",
        "page_break_before_warning_section",
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
        return _court_motion_form_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.34in",
    title="Court Motion Form CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _court_motion_form_document(payload: dict[str, Any]) -> object:
    caption = dict(payload.get("court_caption") or {})
    sig = dict(payload.get("signature_block") or {})
    grounds = [dict(x) for x in (payload.get("grounds") or [])]
    bond_rows = [dict(x) for x in (payload.get("bond_rows") or [])]
    motion_title_lines = [str(x) for x in (payload.get("motion_title_lines") or []) if str(x).strip()]
    opening_statement = str(payload.get("opening_statement") or "").strip()
    grounds_intro = str(payload.get("grounds_intro") or "").strip()
    warning_paragraphs = [str(x) for x in (payload.get("warning_paragraphs") or []) if str(x).strip()]
    service_paragraphs = [str(x) for x in (payload.get("service_certification_paragraphs") or []) if str(x).strip()]
    break_before_warning = bool(payload.get("page_break_before_warning_section"))

    court_caption_heading = str(payload.get("court_caption_heading") or "Court Caption")
    bond_rows_heading = str(payload.get("bond_rows_heading") or "Bond(s) Posted")
    grounds_heading = str(payload.get("grounds_heading") or "Grounds Asserted")
    warning_heading = str(payload.get("warning_heading") or "Hearing Participation and Oath Notice")
    service_heading = str(payload.get("service_certification_heading") or "Service Certification")
    signature_heading = str(payload.get("signature_heading") or "Signature and Contact Information")

    signature_semantics = el(
        "span",
        str(sig.get("signature_text") or "Signature line present"),
        data_fb_a11y_signature_status=str(sig.get("signature_status") or "unknown"),
        data_fb_a11y_signature_method=str(sig.get("signature_method") or "signature_line_only"),
        data_fb_a11y_signature_ref=str(sig.get("signature_ref") or "movant-signature"),
        class_name="cmf-signature-semantics",
    )

    page_one_children: list[Any] = []
    page_two_children: list[Any] = []
    if str(payload.get("header_note") or "").strip():
        page_one_children.append(Text(str(payload.get("header_note")), tag="p", class_name="cmf-header-note"))

    page_one_children.append(
        Section(
            Heading(court_caption_heading, level=1),
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem(
                            "Court",
                            caption.get(
                                "court_line",
                                "IN THE CIRCUIT COURT IN AND FOR ESCAMBIA COUNTY, FLORIDA",
                            ),
                        ),
                        FieldItem("Division", caption.get("division_line", "FAMILY LAW DIVISION")),
                        FieldItem(
                            "Petitioner / Custodial Parent / Designated Relative",
                            caption.get("petitioner", "[Blank on form]"),
                        ),
                        FieldItem(
                            "Respondent / Non-Custodial Parent",
                            caption.get("respondent", "[Blank on form]"),
                        ),
                    ),
                    class_name="cmf-card",
                ),
                Box(
                    FieldGrid(
                        FieldItem("Case No.", caption.get("case_number", "[Blank on form]")),
                        FieldItem("Division", caption.get("division_case_code", "[Blank on form]")),
                    ),
                    class_name="cmf-card",
                ),
                class_name="cmf-caption-grid",
            ),
            class_name="cmf-section",
        )
    )

    if motion_title_lines or opening_statement:
        page_one_children.append(
            Section(
                *[Heading(line, level=2 if i == 0 else 3) for i, line in enumerate(motion_title_lines)],
                *_paragraphs([opening_statement]),
                class_name="cmf-section",
            )
        )

    if bond_rows:
        row_items: list[Any] = []
        for idx, row in enumerate(bond_rows, start=1):
            charge = str(row.get("charge") or "[Blank on form]").strip()
            amount = str(row.get("amount") or "[Blank on form]").strip()
            bond_power_no = str(row.get("bond_power_no") or "[Blank on form]").strip()
            row_items.append(
                FieldItem(
                    str(row.get("label") or f"Bond {idx}"),
                    f"Charge: {charge}; Amount: {amount}; Bond Power No.: {bond_power_no}",
                )
            )
        page_one_children.append(
            Section(
                Heading(bond_rows_heading, level=2),
                Box(FieldGrid(*row_items), class_name="cmf-card"),
                class_name="cmf-section",
            )
        )

    if grounds:
        page_one_children.append(
            FieldSet(
                Legend(grounds_heading),
                *([Text(grounds_intro, tag="p", class_name="cmf-body")] if grounds_intro else []),
                el("ul", *_checkbox_rows(grounds), class_name="cmf-checkbox-list"),
                class_name="cmf-fieldset",
            )
        )

    if warning_paragraphs:
        warning_class = "cmf-section page-break-before" if break_before_warning else "cmf-section"
        warning_node = Section(
            Heading(warning_heading, level=2),
            *_paragraphs(warning_paragraphs),
            class_name=warning_class,
        )
        if break_before_warning:
            page_two_children.append(warning_node)
        else:
            page_one_children.append(warning_node)

    if service_paragraphs:
        service_node = Section(
            Heading(service_heading, level=2),
            *_paragraphs(service_paragraphs),
            class_name="cmf-section",
        )
        if break_before_warning:
            page_two_children.append(service_node)
        else:
            page_one_children.append(service_node)

    signature_node = FieldSet(
        Legend(signature_heading),
        LayoutGrid(
            Box(
                FieldGrid(
                    FieldItem("Dated", sig.get("dated", "[Blank on form]")),
                    FieldItem("Signature of Petitioner/Respondent", signature_semantics),
                    FieldItem("Printed Name", sig.get("printed_name", "[Blank on form]")),
                    FieldItem("Address", sig.get("address", "[Blank on form]")),
                    FieldItem("City, State, Zip", sig.get("city_state_zip", "[Blank on form]")),
                    FieldItem("Telephone/Fax", sig.get("telephone_fax", "[Blank on form]")),
                    FieldItem("Email", sig.get("email", "[Blank on form]")),
                ),
                class_name="cmf-card",
            ),
            class_name="cmf-signature-grid",
        ),
        class_name="cmf-fieldset",
    )

    if break_before_warning:
        page_two_children.append(signature_node)
    else:
        page_one_children.append(signature_node)

    if page_two_children:
        return Stack(
            Region(*page_one_children, label="Court motion/application form page 1", class_name="cmf-page cmf-page-1"),
            Region(
                *page_two_children,
                label="Court motion/application form page 2",
                class_name="cmf-page cmf-page-2",
                style={"break-before": "page", "page-break-before": "always"},
            ),
            class_name="cmf-root",
        )

    return Stack(*page_one_children, class_name="cmf-root")


__all__ = [
    "COURT_MOTION_FORM_FAMILY_ID",
    "FL_ESCAMBIA_COURT_MOTION_FORM_CHILD_SUPPORT_TELEPHONE_HEARING_TITLE_IV_D_2019_V1",
    "FL_ESCAMBIA_COURT_MOTION_FORM_CLERK_DISCHARGE_FORFEITURE_FS903_26_8_2020_V1",
    "CourtMotionFormCavKit",
]
