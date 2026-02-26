from __future__ import annotations

import pytest

from fullbleed.ui import cav


def test_public_cav_surface_exports_family_kits_and_profiles_namespace() -> None:
    assert hasattr(cav, "AgencyLetterCavKit")
    assert hasattr(cav, "CourtMotionFormCavKit")
    assert hasattr(cav, "DeclarationFormCavKit")
    assert hasattr(cav, "InstructionSheetCavKit")
    assert hasattr(cav, "InvestmentPortfolioReportCavKit")
    assert hasattr(cav, "MarriageRecordCavKit")
    assert hasattr(cav, "RecordedPlatCavKit")
    assert hasattr(cav, "RequestRedactionFormCavKit")
    assert hasattr(cav, "WarrantyDeedCavKit")
    assert hasattr(cav, "cav_profiles")


def test_profile_registry_supports_lookup_and_family_filtering() -> None:
    p = cav.cav_profiles.REGISTRY.by_id("fl.escambia.marriage_record.rev2019")
    assert p.family_id == "marriage_record_cav"
    ids = cav.cav_profiles.REGISTRY.list_ids(family_id="marriage_record_cav")
    assert "fl.escambia.marriage_record.rev2019" in ids
    assert "fl.escambia.warranty_deed.rev1994" not in ids
    letter = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.agency_notice_letter.single_page_clerk_letterhead_notice.v1"
    )
    assert letter.family_id == "agency_letter_cav"
    public_notice = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.agency_notice.public_notice_vab_rescheduled_2020.v1"
    )
    assert public_notice.family_id == "agency_letter_cav"
    decl = cav.cav_profiles.REGISTRY.by_id("fl.escambia.declaration_form.cdc_eviction_2020_layout.v1")
    assert decl.family_id == "declaration_form_cav"
    motion = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.court_motion_form.child_support_telephone_hearing_title_iv_d_2019.v1"
    )
    assert motion.family_id == "court_motion_form_cav"
    forfeiture = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.court_motion_form.clerk_discharge_forfeiture_fs903_26_8_2020.v1"
    )
    assert forfeiture.family_id == "court_motion_form_cav"
    instr = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.instruction_sheet.child_support_phone_testimony_2019.v1"
    )
    assert instr.family_id == "instruction_sheet_cav"
    packet = cav.cav_profiles.REGISTRY.by_id(
        "fl.state.family_law_instruction_packet.form12_961_notice_hearing_contempt_support_2018.v1"
    )
    assert packet.family_id == "instruction_sheet_cav"
    packet_12921 = cav.cav_profiles.REGISTRY.by_id(
        "fl.state.family_law_instruction_packet.form12_921_notice_hearing_child_support_enforcement_2018.v1"
    )
    assert packet_12921.family_id == "instruction_sheet_cav"
    investment = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.investment_portfolio_summary.fy2019_2020.nov2019.v1"
    )
    assert investment.family_id == "investment_portfolio_report_cav"
    redaction = cav.cav_profiles.REGISTRY.by_id(
        "fl.escambia.request_redaction_exempt_personal_information.effective_2025.v1"
    )
    assert redaction.family_id == "request_redaction_form_cav"
    plat = cav.cav_profiles.REGISTRY.by_id("fl.escambia.recorded_plat.legacy_side_certificate_layout.v1")
    assert plat.family_id == "recorded_plat_cav"


def test_family_kit_rejects_mismatched_profile_family() -> None:
    with pytest.raises(ValueError):
        cav.MarriageRecordCavKit(profile=cav.cav_profiles.FL_ESCAMBIA_WARRANTY_DEED_REV1994)
    with pytest.raises(ValueError):
        cav.AgencyLetterCavKit(profile=cav.cav_profiles.FL_ESCAMBIA_WARRANTY_DEED_REV1994)


def test_family_kit_scope_validation_reports_unmapped_fields() -> None:
    kit = cav.WarrantyDeedCavKit(profile=cav.cav_profiles.FL_ESCAMBIA_WARRANTY_DEED_REV1994)
    report = kit.validate_payload_scope({"page1": {}, "unexpected_field": 1})
    assert report["ok"] is False
    assert report["issues"]
    issue = report["issues"][0]
    assert issue["code"] == "CAVKIT_PROFILE_UNMAPPED_PAYLOAD_FIELD"
    assert issue["severity"] == "error"
    assert "unexpected_field" in issue["fields"]


def test_family_kit_render_shape_is_props_first_and_scope_checked_before_render() -> None:
    kit = cav.MarriageRecordCavKit(profile=cav.cav_profiles.FL_ESCAMBIA_MARRIAGE_RECORD_REV2019)
    with pytest.raises(ValueError):
        kit.render(payload={"unexpected": 1}, claim_evidence={"profile": {}})


def test_warranty_deed_family_kit_renders_document_artifact() -> None:
    kit = cav.WarrantyDeedCavKit(profile=cav.cav_profiles.FL_ESCAMBIA_WARRANTY_DEED_REV1994)
    payload = {
        "title": "This Warranty Deed",
        "header": {
            "instrument_book_page": "OR Bk0000 Pg0000",
            "instrument_number": "INSTRUMENT 00000000",
            "margin_marking": "10.50",
            "prepared_by_lines": ["PREPARED BY: Test"],
            "file_number": "File No: T-0000",
        },
        "warranty_deed": {
            "execution_date_text": "Made this 1st day of January, A.D. 2000",
            "grantors": "Grantor A",
            "grantees": "Grantee B",
            "property_address_lines": ["123 Test St", "Pensacola, FL"],
            "grant_text_paragraphs": ["Witnesseth..."],
            "parcel_identification_number": "000000000000000000",
            "habendum_and_warranty_paragraphs": ["To Have and to Hold..."],
        },
        "recorder_markings": {
            "page1_box": {
                "dsp_d": "$0.00",
                "date_stamp": "JAN 01 2000",
                "clerk_name": "CLERK",
                "clerk_role": "COMPTROLLER",
                "by_line": "[Illegible]",
                "cert_reg": "CERT. REG. #0",
            }
        },
        "witness_and_grantor_signatures": {
            "presence_statement": "Signed, sealed and delivered in our presence:",
            "witness_rows": [
                {
                    "slot": "Witness 1",
                    "witness_name": "Witness Name",
                    "signature_status": "present",
                    "signature_method": "wet_ink_scan",
                    "signature_text": "Signature present: Witness Name",
                    "reference_id": "w1",
                    "name_address_line": "[Address line]",
                }
            ],
            "grantor_rows": [
                {
                    "slot": "Grantor 1",
                    "printed_name": "Grantor A",
                    "signature_status": "present",
                    "signature_method": "wet_ink_scan",
                    "signature_text": "Signature present: Grantor A",
                    "reference_id": "g1",
                    "name_address_line": "[Address line]",
                }
            ],
        },
        "notary_acknowledgment": {
            "state": "FLORIDA",
            "county": "ESCAMBIA",
            "ack_date_text": "1st day of January, 2000",
            "acknowledging_parties": "Grantor A",
            "identity_clause": "personally known to me",
            "notary_signature_status": "present",
            "notary_signature_method": "wet_ink_scan",
            "notary_signature_text": "Signature present: Notary",
            "notary_signature_ref": "notary",
            "print_name_line": "Notary Name",
            "commission_expires_line": "01/01/2004",
            "official_seal_present": True,
            "official_seal_text_visible": "OFFICIAL SEAL",
        },
        "schedule_a": {
            "title": "Schedule A",
            "instrument_ref_header": "OR Bk0000 Pg0001 / INSTRUMENT 00000000",
            "legal_description_text": "Lot 1, Block 1, Test Subdivision.",
            "stamp_lines": ["Instrument 00000000"],
        },
        "page3": {
            "file_reference": "File No: T-0000",
            "horizontal_line_present": True,
            "body_content_note": "Blank page note.",
        },
        "signatures": {},
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "This Warranty Deed" in html
    assert "Schedule A" in html


def test_recorded_plat_family_kit_renders_document_artifact() -> None:
    kit = cav.RecordedPlatCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1
    )
    payload = {
        "schema": "fullbleed.cav.plat.v1",
        "document_kind": "recorded_plat",
        "title": "RENZ-ANNA VILLA",
        "subtitle_lines": ["ESCAMBIA COUNTY, FLORIDA"],
        "plat_metadata": {
            "jurisdiction": "FLORIDA",
            "county": "Escambia",
            "plat_book_page": "PB 1 PG 85",
            "sheet_notation": "PG 85",
            "margin_marking": "PB 1 PG 85",
            "prepared_by": "CAMPBELL & CHOVERMAN, PENSACOLA, FLORIDA",
        },
        "plan_image": {
            "src": "C:/tmp/plat.png",
            "alt": "Recorded plat map image for Renz-Anna Villa.",
            "caption": "Source plat image.",
        },
        "recording_annotations": {
            "summary": "Recorded plat page with dedication and certificate blocks.",
            "visible_blocks": ["Dedication", "Engineer's Certificate", "Clerk's Certificate"],
        },
        "certificate_blocks": [
            {
                "block": "Dedication",
                "heading": "DEDICATION",
                "summary": "Owners dedicate rights-of-way and easements as shown.",
                "signer_line": "Signatures present (seal lines visible)",
                "seal_present": True,
            }
        ],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "RENZ-ANNA VILLA" in html
    assert "Recorded Plat Map" in html


def test_declaration_form_family_kit_renders_document_artifact() -> None:
    kit = cav.DeclarationFormCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_DECLARATION_FORM_CDC_EVICTION_2020_LAYOUT_V1
    )
    payload = {
        "schema": "fullbleed.cav.declaration.v1",
        "document_kind": "declaration_form",
        "title": "DECLARATION UNDER PENALTY OF PERJURY",
        "subtitle_lines": ["CDC TEMPORARY HALT IN EVICTIONS"],
        "intro_paragraphs": ["This declaration is for tenants..."],
        "declaration_lead": "I certify under penalty of perjury...",
        "statement_blocks": [{"prompt": "Statement one", "response_line_count": 2}],
        "initials_blocks": [{"text": "I understand this statement."}],
        "signature_block": {
            "signature_text": "Signature line present",
            "signature_status": "signature_line_only",
            "signature_method": "signature_line_only",
            "signature_ref": "decl-sig",
            "date": "[Blank on form]",
            "print_name": "[Blank on form]",
            "phone": "[Blank on form]",
            "email": "[Blank on form]",
            "address": "[Blank on form]",
        },
        "authority_footer": ["Authority: ...", "Dated: September 1, 2020."],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "DECLARATION UNDER PENALTY OF PERJURY" in html
    assert "Declarant Signature and Contact" in html


def test_court_motion_form_family_kit_renders_document_artifact() -> None:
    kit = cav.CourtMotionFormCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_COURT_MOTION_FORM_CHILD_SUPPORT_TELEPHONE_HEARING_TITLE_IV_D_2019_V1
    )
    payload = {
        "schema": "fullbleed.cav.court_motion_form.v1",
        "document_kind": "court_motion_form",
        "header_note": "Updated 6/2019 by Child Support Hearing Officer",
        "court_caption": {
            "court_line": "IN THE CIRCUIT COURT IN AND FOR ESCAMBIA COUNTY, FLORIDA",
            "division_line": "FAMILY LAW DIVISION",
            "petitioner": "[Blank on form]",
            "respondent": "[Blank on form]",
            "case_number": "[Blank on form]",
            "division_case_code": "[Blank on form]",
        },
        "motion_title_lines": [
            "MOTION FOR AUTHORITY TO PARTICIPATE/TESTIFY BY TELEPHONE",
            "IN CHILD SUPPORT CASE HEARING (TITLE IV-D CASE)",
        ],
        "opening_statement": "I move for authority to participate by telephone in the child support case hearing.",
        "grounds_intro": "The motion is based on the following grounds:",
        "grounds": [{"checked": False, "text": "I reside outside the State of Florida."}],
        "warning_paragraphs": ["A notary must be present to verify identity and administer the oath if testifying."],
        "service_certification_paragraphs": ["I certify that a copy of this motion has been served as indicated."],
        "signature_block": {
            "signature_text": "Signature line present",
            "signature_status": "unknown",
            "signature_method": "signature_line_only",
            "signature_ref": "motion-sig",
            "dated": "[Blank on form]",
            "printed_name": "[Blank on form]",
            "address": "[Blank on form]",
            "city_state_zip": "[Blank on form]",
            "telephone_fax": "[Blank on form]",
            "email": "[Blank on form]",
        },
        "metadata": {},
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "MOTION FOR AUTHORITY TO PARTICIPATE/TESTIFY BY TELEPHONE" in html
    assert "Court Caption" in html


def test_court_motion_form_supports_forfeiture_shape_extensions() -> None:
    kit = cav.CourtMotionFormCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_COURT_MOTION_FORM_CLERK_DISCHARGE_FORFEITURE_FS903_26_8_2020_V1
    )
    payload = {
        "schema": "fullbleed.cav.court_motion_form.v1",
        "document_kind": "court_motion_form",
        "header_note": "Revised 07/27/2020",
        "court_caption_heading": "Case Caption",
        "court_caption": {
            "court_line": "IN THE _________________ COURT OF THE FIRST JUDICIAL CIRCUIT IN AND FOR ESCAMBIA COUNTY, FLORIDA",
            "division_line": "STATE OF FLORIDA",
            "petitioner": "STATE OF FLORIDA",
            "respondent": "[Blank on form] / Defendant",
            "case_number": "[Blank on form]",
            "division_case_code": "[Blank on form]",
        },
        "motion_title_lines": [
            "APPLICATION FOR CLERK'S DISCHARGE OF FORFEITURE",
            "(F.S. 903.26(8))",
        ],
        "opening_statement": "1. I, [Blank on form], the bail bond agent, posted the following bond(s).",
        "bond_rows_heading": "Bond(s) Posted",
        "bond_rows": [
            {"label": "Bond 1", "charge": "[Blank]", "amount": "[Blank]", "bond_power_no": "[Blank]"},
        ],
        "grounds_heading": "Statutory Basis",
        "grounds_intro": "6. The forfeiture should be discharged because (check the appropriate box):",
        "grounds": [{"checked": False, "text": "Defendant was arrested or surrendered in Escambia County."}],
        "warning_heading": "Forfeiture and Timing Statements",
        "warning_paragraphs": ["2. The Defendant failed to appear in court on [Blank date]."],
        "service_certification_heading": "Declaration Under Penalties of Perjury",
        "service_certification_paragraphs": [
            "Under penalties of perjury, I declare the foregoing application is true to the best of my knowledge and belief."
        ],
        "page_break_before_warning_section": True,
        "signature_heading": "Petitioner/Bail Bond Agent Signature and Contact Information",
        "signature_block": {
            "signature_text": "Signature line present (blank on source form)",
            "signature_status": "missing",
            "signature_method": "signature_line_only",
            "signature_ref": "doc728-signature",
            "dated": "[Blank on form]",
            "printed_name": "[Blank on form]",
            "address": "[Blank on form]",
            "city_state_zip": "[Blank on form]",
            "telephone_fax": "[Blank on form]",
            "email": "[Blank on form]",
        },
        "metadata": {},
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "APPLICATION FOR CLERK" in html
    assert "DISCHARGE OF FORFEITURE" in html
    assert "Bond(s) Posted" in html
    assert "page-break-before" in html


def test_agency_letter_family_kit_renders_document_artifact() -> None:
    kit = cav.AgencyLetterCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_2021_V1
    )
    payload = {
        "schema": "fullbleed.cav.agency_letter.v1",
        "document_kind": "agency_notice_letter",
        "title": "Tourist Development Tax (TDT) increases to 5%, effective April 2021",
        "letterhead": {
            "seal_text": "Official seal emblem present",
            "agency_name": "Pam Childers",
            "office_name": "Clerk of the Circuit Court and Comptroller, Escambia County",
            "suboffice_line": "Clerk of Courts • County Comptroller • Recorder • Auditor",
        },
        "date_line": "February 1, 2021",
        "subject_line": "Tourist Development Tax (TDT) increases to 5%, effective April 2021",
        "salutation": "Dear Tourist Development Taxpayer,",
        "paragraphs": [
            {
                "segments": [
                    {"text": "Our records indicate that you are currently collecting or have previously collected Tourist Development Tax (TDT) in Escambia County."}
                ]
            }
        ],
        "resource_links": [
            {
                "label": "Ordinance",
                "text": "Escambia County Ordinance Sec. 90-65",
                "href": "https://example.invalid/ordinance",
            }
        ],
        "closing_block": {
            "closing": "Sincerely,",
            "signer_name": "Escambia County Clerk of the Circuit Court and Comptroller",
            "signer_title": "[Typed signoff]",
        },
        "footer_lines": ["Finance • 221 Palafox Place • Suite 110 • Pensacola, FL 32502"],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "Tourist Development Tax (TDT) increases to 5%" in html
    assert "Escambia County Ordinance Sec. 90-65" in html


def test_agency_letter_family_kit_supports_notice_without_letterhead() -> None:
    kit = cav.AgencyLetterCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_AGENCY_NOTICE_PUBLIC_NOTICE_VAB_RESCHEDULED_2020_V1
    )
    payload = {
        "schema": "fullbleed.cav.agency_letter.v1",
        "document_kind": "agency_notice_letter",
        "show_letterhead": False,
        "document_heading": "PUBLIC NOTICE",
        "paragraphs": [
            "A rescheduled Meeting of the Escambia County Value Adjustment Board (VAB) will be held on Monday, January 13, 2020, at 9:00 a.m., in Board Chambers, First Floor, 221 Palafox Place, Pensacola, FL 32502.",
            "At this Meeting, the VAB will consider the Special Magistrates' recommendations for 2019 Petitions.",
            "Dated: December 16, 2019",
        ],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "PUBLIC NOTICE" in html
    assert "Value Adjustment Board" in html
    assert "letterhead-card" not in html


def test_instruction_sheet_family_kit_renders_document_artifact() -> None:
    kit = cav.InstructionSheetCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_INSTRUCTION_SHEET_CHILD_SUPPORT_PHONE_TESTIMONY_2019_V1
    )
    payload = {
        "schema": "fullbleed.cav.instruction_sheet.v1",
        "document_kind": "instruction_sheet",
        "header_note": "Updated 6/2019 by Child Support Hearing Officer",
        "title_lines": [
            "MOTION FOR AUTHORITY TO PARTICIPATE/TESTIFY BY TELEPHONE",
            "IN CHILD SUPPORT CASE HEARING (TITLE IV-D CASE) INSTRUCTIONS",
        ],
        "division_line": "FAMILY LAW DIVISION - ESCAMBIA COUNTY",
        "metadata_fields": [{"label": "Division", "value": "Family Law Division"}],
        "sections": [
            {
                "heading": "I. You should use this form if the following is ALL true:",
                "items": [
                    "Your hearing is in Escambia County.",
                    "Your hearing is before a Child Support Hearing Officer.",
                ],
            }
        ],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "CHILD SUPPORT CASE HEARING" in html
    assert "Escambia County" in html


def test_instruction_sheet_family_kit_renders_paginated_instruction_packet() -> None:
    kit = cav.InstructionSheetCavKit(
        profile=cav.cav_profiles.FL_STATE_FAMILY_LAW_INSTRUCTION_PACKET_FORM_12_961_2018_V1
    )
    payload = {
        "schema": "fullbleed.cav.instruction_sheet.v1",
        "document_kind": "instruction_sheet",
        "header_note": "Florida Supreme Court Approved Family Law Form 12.961 (09/18)",
        "title_lines": [
            "INSTRUCTIONS FOR FLORIDA SUPREME COURT APPROVED FAMILY LAW FORM 12.961",
            "NOTICE OF HEARING ON MOTION FOR CONTEMPT/ENFORCEMENT",
        ],
        "division_line": "IN SUPPORT MATTERS (RULE 12.615)",
        "running_header": "Instructions for Florida Supreme Court Approved Family Law Form 12.961 (09/18)",
        "pages": [
            {
                "page_label": "Instruction packet page 1",
                "sections": [
                    {
                        "heading": "When should this form be used?",
                        "paragraphs": ["Use this form anytime you have set a hearing on a Motion for Contempt/Enforcement."],
                    }
                ],
            },
            {
                "page_label": "Instruction packet page 2",
                "sections": [
                    {
                        "paragraphs": ["IMPORTANT INFORMATION REGARDING E-SERVICE ELECTION"],
                    }
                ],
            },
        ],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "INSTRUCTIONS FOR FLORIDA SUPREME COURT APPROVED FAMILY LAW FORM 12.961" in html
    assert "instruction-page-break" in html


def test_investment_portfolio_report_family_kit_renders_document_artifact() -> None:
    kit = cav.InvestmentPortfolioReportCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_INVESTMENT_PORTFOLIO_SUMMARY_FY2019_2020_NOV2019_V1
    )
    payload = {
        "schema": "fullbleed.cav.investment_portfolio_report.v1",
        "document_kind": "investment_portfolio_report",
        "cover_page": {
            "page_label": "Cover",
            "title_lines": [
                "INVESTMENT PORTFOLIO SUMMARY REPORT",
                "ESCAMBIA COUNTY BOARD OF COUNTY COMMISSIONERS",
            ],
            "subtitle_lines": ["FISCAL YEAR 2019-2020", "November 30, 2019"],
            "prepared_by_lines": ["Prepared by: Pam Childers"],
            "footer_note": "Prepared by the Clerk of the Circuit Court and Comptroller",
        },
        "pages": [
            {
                "page_label": "Summary allocation",
                "title": "INVESTMENT PORTFOLIO COMPOSITION",
                "sections": [
                    {
                        "heading": "Summary of Investment Allocation",
                        "tables": [
                            {
                                "caption": "October vs November values",
                                "columns": ["Investment Category", "October 31, 2019", "November 30, 2019"],
                                "rows": [
                                    ["Bank Accounts", "24628107", "31816178"],
                                    ["Total Portfolio Assets", "300871187", "308211135"],
                                ],
                            }
                        ],
                    }
                ],
            }
        ],
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "INVESTMENT PORTFOLIO SUMMARY REPORT" in html
    assert "Summary of Investment Allocation" in html


def test_request_redaction_form_family_kit_renders_document_artifact() -> None:
    kit = cav.RequestRedactionFormCavKit(
        profile=cav.cav_profiles.FL_ESCAMBIA_REQUEST_REDACTION_EXEMPT_PERSONAL_INFORMATION_EFFECTIVE_2025_V1
    )
    payload = {
        "schema": "fullbleed.cav.request_redaction_form.v1",
        "document_kind": "request_redaction_form",
        "title_lines": [
            "REQUEST FOR REDACTION OF EXEMPT PERSONAL INFORMATION",
            "FROM NON-JUDICIAL PUBLIC RECORDS",
            "EFFECTIVE MARCH 19, 2025",
        ],
        "intro_paragraphs": [
            "I request to have exempt personal information removed from records maintained by the Escambia County Clerk of the Circuit Court and Comptroller's Office.",
        ],
        "statutory_categories_left": [
            "Victim of a violent crime.",
            "Human trafficking victim.",
        ],
        "statutory_categories_right": [
            "Emergency medical technician or paramedic.",
            "Judge or justice.",
        ],
        "category_note": "Grantor, grantee, or party names cannot be removed unless they contain the street address.",
        "requestor_contact": {
            "printed_name": "[Blank on form]",
            "telephone": "[Blank on form]",
            "email": "[Blank on form]",
        },
        "information_to_be_redacted": {
            "residence_address": "[Blank on form]",
            "additional_address_descriptions": "[Blank on form]",
            "telephone_numbers": "[Blank on form]",
            "ssn_dob": "[Blank on form]",
            "spouse_children_names": "[Blank on form]",
            "employment_location": "[Blank on form]",
            "school_daycare_location": "[Blank on form]",
            "personal_assets": "[Blank on form]",
        },
        "warning_paragraphs": [
            "WARNING: There may be consequences to redacting information on a public record.",
            "PUBLIC RECORD: This form is itself a public record.",
        ],
        "documents_intro_paragraphs": [
            "The following section is to be completed during or after a visit to the Escambia Clerk office.",
        ],
        "documents_table_rows": [
            {
                "instrument_number": "[Blank]",
                "book": "[Blank]",
                "page": "[Blank]",
                "document_title": "[Blank]",
            }
        ],
        "documents_other_line": "[Blank on form]",
        "release_to_government_paragraphs": [
            "RELEASE TO GOVERNMENTAL AGENCIES: An unredacted version of these documents will be provided as required.",
        ],
        "release_for_title_searches_paragraphs": [
            "RELEASE FOR TITLE SEARCHES: An unredacted version may be provided to title insurers or agents as authorized.",
        ],
        "courtesy_notice_paragraphs": [
            "If you have previously requested protection of a home address that is no longer your residence, submit a written request to release the removed information.",
        ],
        "notary_block": {
            "state": "FLORIDA",
            "county": "ESCAMBIA",
            "sworn_statement": "[Blank on form]",
            "identity_line": "[Blank on form]",
            "notary_signature_line": "[Blank on form]",
            "notary_print_name": "[Blank on form]",
        },
        "signature_block": {
            "signature_status": "missing",
            "signer_name": "Requestor",
            "signature_method": "signature_line_only",
            "reference_id": "requestor-signature",
        },
        "review_queue": [],
        "metadata": {},
        "source_pdf": None,
    }
    artifact = kit.render(payload=payload, claim_evidence={})
    html = artifact.root.to_html()
    assert "REQUEST FOR REDACTION OF EXEMPT PERSONAL INFORMATION" in html
    assert "Documents to be redacted" in html
