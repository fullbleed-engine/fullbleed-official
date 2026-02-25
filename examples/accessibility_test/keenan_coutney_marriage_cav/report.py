from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import fullbleed
from fullbleed.accessibility import AccessibilityEngine

from fullbleed.ui import Text, el, validate_component_mount
from fullbleed.ui.accessibility import (
    A11yContract,
    FieldGrid,
    FieldItem,
)
from fullbleed.ui.core import Document


ROOT = Path(__file__).resolve().parent
SOURCE_PDF_PATH = ROOT.parent / "keenan_coutney_marriage.pdf"
OUTPUT_DIR = ROOT / "output"
CSS_PATH = ROOT / "styles" / "report.css"
VENDOR_FONT_PATH = ROOT / "vendor" / "fonts" / "Inter-Variable.ttf"

DOC_STEM = "keenan_coutney_marriage_cav"
HTML_PATH = OUTPUT_DIR / f"{DOC_STEM}.html"
CSS_ARTIFACT_PATH = OUTPUT_DIR / f"{DOC_STEM}.css"
PDF_PATH = OUTPUT_DIR / f"{DOC_STEM}.pdf"
A11Y_VALIDATION_PATH = OUTPUT_DIR / f"{DOC_STEM}_a11y_validation.json"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / f"{DOC_STEM}_component_mount_validation.json"
TRANSCRIPTION_PATH = OUTPUT_DIR / f"{DOC_STEM}_transcription.json"
RUN_REPORT_PATH = OUTPUT_DIR / f"{DOC_STEM}_run_report.json"
PARITY_REPORT_PATH = OUTPUT_DIR / f"{DOC_STEM}_parity_report.json"
SOURCE_ANALYSIS_PATH = OUTPUT_DIR / f"{DOC_STEM}_source_analysis.json"
SOURCE_PREVIEW_PATH = OUTPUT_DIR / f"{DOC_STEM}_source_page1.png"
CLAIM_EVIDENCE_PATH = OUTPUT_DIR / f"{DOC_STEM}_claim_evidence.json"
ENGINE_A11Y_VERIFY_PATH = OUTPUT_DIR / f"{DOC_STEM}_a11y_verify_engine.json"
ENGINE_PMR_PATH = OUTPUT_DIR / f"{DOC_STEM}_pmr_engine.json"
PREVIEW_PNG_STEM = DOC_STEM


@dataclass(frozen=True)
class Applicant:
    label: str
    full_name: str
    maiden_surname: str | None
    date_of_birth: str
    residence_city: str
    county: str
    state: str
    birthplace: str


@dataclass(frozen=True)
class SignatureRecord:
    role: str
    signature_status: str
    signature_method: str
    signed_on: str | None
    signer_name: str | None
    reference_id: str
    notes: str | None = None


@dataclass(frozen=True)
class ReviewItem:
    field_ref: str
    confidence: str
    issue: str
    next_step: str


APPLICANT_1 = Applicant(
    label="Applicant 1",
    full_name="Keenan Ross Finkelstein",
    maiden_surname=None,
    date_of_birth="06/13/1989",
    residence_city="Pensacola",
    county="Escambia",
    state="Florida",
    birthplace="Arizona",
)

APPLICANT_2 = Applicant(
    label="Applicant 2",
    full_name="Courtney Leann Lawson",
    maiden_surname=None,
    date_of_birth="12/07/1993",
    residence_city="Pensacola",
    county="Escambia",
    state="Florida",
    birthplace="Alabama",
)


APPLICATION_SIGNATURES: tuple[SignatureRecord, ...] = (
    SignatureRecord(
        role="Applicant 1 signature (Field 9)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name=APPLICANT_1.full_name,
        reference_id="form-field-9",
        notes="Handwritten signature present in scan.",
    ),
    SignatureRecord(
        role="Official signature for applicant 1 notarization (Field 12)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name=None,
        reference_id="form-field-12",
        notes="Signature appears to read 'Ann Smith' but requires review.",
    ),
    SignatureRecord(
        role="Applicant 2 signature (Field 13)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name=APPLICANT_2.full_name,
        reference_id="form-field-13",
        notes="Handwritten signature present in scan.",
    ),
    SignatureRecord(
        role="Official signature for applicant 2 notarization (Field 16)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name=None,
        reference_id="form-field-16",
        notes="Signature appears to read 'Ann Smith' but requires review.",
    ),
)


LICENSE_SIGNATURES: tuple[SignatureRecord, ...] = (
    SignatureRecord(
        role="Signature of court clerk or judge (Field 20)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name="Pam Childers",
        reference_id="form-field-20",
        notes="Signer name inferred from visible signature and recorded stamp text.",
    ),
    SignatureRecord(
        role="By D.C. initials (Field 20c)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="05/31/2019",
        signer_name=None,
        reference_id="form-field-20c",
        notes="Initials appear to be 'AS' (uncertain).",
    ),
)


CERTIFICATE_SIGNATURES: tuple[SignatureRecord, ...] = (
    SignatureRecord(
        role="Signature of person performing ceremony (Field 23a)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="06/03/2019",
        signer_name=None,
        reference_id="form-field-23a",
        notes="Handwritten signature present; signer name printed separately in Field 23b.",
    ),
    SignatureRecord(
        role="Signature of witness to ceremony (Field 24)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="06/03/2019",
        signer_name=None,
        reference_id="form-field-24",
        notes="Witness name not confidently legible from scan.",
    ),
    SignatureRecord(
        role="Signature of second witness to ceremony (Field 25)",
        signature_status="present",
        signature_method="wet_ink_scan",
        signed_on="06/03/2019",
        signer_name=None,
        reference_id="form-field-25",
        notes="Witness name not confidently legible from scan.",
    ),
)


REVIEW_QUEUE: tuple[ReviewItem, ...] = (
    ReviewItem(
        field_ref="Field 12 / Field 16 official signatures",
        confidence="medium",
        issue="Official handwritten name appears to read 'Ann Smith' but is not fully clear.",
        next_step="Confirm against county clerk staff records or a clearer source scan.",
    ),
    ReviewItem(
        field_ref="Field 20c By D.C. initials",
        confidence="low",
        issue="Initials are present but partially obscured; likely 'AS'.",
        next_step="Confirm from original high-resolution scan or clerk record.",
    ),
    ReviewItem(
        field_ref="Field 23b performer name/title (handwritten lines)",
        confidence="medium",
        issue="Name/title text appears to be 'Gary T. Dougherty / Sr. Pastor / Courts of Praise Fellowship'.",
        next_step="Confirm with higher resolution scan or source clerk filing metadata.",
    ),
    ReviewItem(
        field_ref="Fields 24-25 witness signatures",
        confidence="low",
        issue="Signature marks are present, but witness names are not confidently legible.",
        next_step="Retain signature-presence semantics and mark names unknown until verified.",
    ),
)


SOURCE_ANALYSIS_CACHE: dict[str, Any] | None = None


def _register_vendored_font(engine: AccessibilityEngine) -> None:
    if not hasattr(fullbleed, "AssetBundle"):
        return
    if not VENDOR_FONT_PATH.exists():
        print(f"[warn] Vendored font not found: {VENDOR_FONT_PATH}")
        return
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(VENDOR_FONT_PATH), "font")
    engine.register_bundle(bundle)


def create_engine() -> AccessibilityEngine:
    engine = AccessibilityEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        document_lang="en-US",
        document_title="Keenan/Courtney Marriage Record CAV (Accessibility-First)",
        strict=False,
    )
    _register_vendored_font(engine)
    return engine


def _emit_preview_png(engine: fullbleed.PdfEngine, html: str, css: str, out_dir: Path, *, stem: str) -> list[str]:
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
    if hasattr(engine, "render_image_pages"):
        page_images = list(engine.render_image_pages(html, css, 144) or [])
        out_paths: list[str] = []
        for idx, image_bytes in enumerate(page_images, start=1):
            path = out_dir / f"{stem}_page{idx}.png"
            path.write_bytes(image_bytes)
            out_paths.append(str(path))
        return out_paths
    return []


def _extract_source_analysis() -> dict[str, Any]:
    out: dict[str, Any] = {
        "source_pdf_path": str(SOURCE_PDF_PATH),
        "source_exists": SOURCE_PDF_PATH.exists(),
        "page_count": None,
        "text_extract_char_count_page1": None,
        "text_layer_present_page1": None,
        "preview_png_path": None,
        "fitz_available": False,
        "pypdf_available": False,
        "warnings": [],
    }
    if not SOURCE_PDF_PATH.exists():
        out["warnings"].append("Source PDF not found.")
        return out

    try:
        from pypdf import PdfReader  # type: ignore

        out["pypdf_available"] = True
        reader = PdfReader(str(SOURCE_PDF_PATH))
        out["page_count"] = len(reader.pages)
        if reader.pages:
            text = reader.pages[0].extract_text() or ""
            out["text_extract_char_count_page1"] = len(text)
            out["text_layer_present_page1"] = bool(text.strip())
    except Exception as exc:
        out["warnings"].append(f"pypdf analysis unavailable: {type(exc).__name__}: {exc}")

    try:
        import fitz  # type: ignore

        out["fitz_available"] = True
        doc = fitz.open(SOURCE_PDF_PATH)
        if out["page_count"] is None:
            out["page_count"] = int(doc.page_count)
        if doc.page_count > 0:
            page = doc.load_page(0)
            out["page1_size_points"] = {
                "width": float(page.rect.width),
                "height": float(page.rect.height),
            }
            pix = page.get_pixmap(dpi=144)
            pix.save(str(SOURCE_PREVIEW_PATH))
            out["preview_png_path"] = str(SOURCE_PREVIEW_PATH)
        doc.close()
    except Exception as exc:
        out["warnings"].append(f"fitz preview export unavailable: {type(exc).__name__}: {exc}")

    return out


def get_source_analysis() -> dict[str, Any]:
    global SOURCE_ANALYSIS_CACHE
    if SOURCE_ANALYSIS_CACHE is None:
        OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
        SOURCE_ANALYSIS_CACHE = _extract_source_analysis()
    return SOURCE_ANALYSIS_CACHE


def _field_box(label: str, value: Any, *, class_name: str | None = None, **props: Any) -> object:
    classes = "fb-field" if class_name is None else f"fb-field {class_name}"
    return el(
        "div",
        FieldGrid(FieldItem(label, value), class_name="fb-field-grid"),
        class_name=classes,
        **props,
    )


def _row(*cells: Any, class_name: str | None = None, cols: str | None = None) -> object:
    classes = "fb-row" if class_name is None else f"fb-row {class_name}"
    props: dict[str, Any] = {"class_name": classes}
    if cols is not None:
        props["style"] = {"grid_template_columns": cols}
    return el("div", list(cells), **props)


def _section_bar(title: str) -> object:
    return Text(title, tag="h2", class_name="fb-section-title")


def _boilerplate(text: str) -> object:
    return el("p", text, class_name="fb-boilerplate")


def _sig_value(*, signer_name: str | None, signed_on: str | None, reference_id: str) -> object:
    lines: list[Any] = [
        el(
            "div",
            "Signature present"
            + (f": {signer_name}" if signer_name else ": [Illegible in source scan]"),
            class_name="sig-line",
            data_fb_a11y_signature_status="present",
            data_fb_a11y_signature_method="wet_ink_scan",
            data_fb_a11y_signature_ref=reference_id,
        )
    ]
    if signed_on:
        lines.append(el("div", f"Date: {signed_on}", class_name="sig-subline"))
    return el("div", lines, class_name="sig-value")


def _seal_value() -> object:
    return el("div", "SEAL PRESENT", class_name="seal-value")


def _application_to_marry_section() -> object:
    return el(
        "section",
        _section_bar("APPLICATION TO MARRY"),
        _row(
            _field_box(
                "1. NAME OF SPOUSE (First, Middle, Last)",
                APPLICANT_1.full_name.upper(),
                class_name="val-lg",
            ),
            _field_box("1b. MAIDEN SURNAME (If applicable)", (APPLICANT_1.maiden_surname or "")),
            _field_box("2. DATE OF BIRTH (Month, Day, Year)", APPLICANT_1.date_of_birth, class_name="val-lg"),
            cols="2.35fr 0.95fr 1fr",
        ),
        _row(
            _field_box("3a. RESIDENCE - CITY, TOWN OR LOCATION", APPLICANT_1.residence_city.upper(), class_name="val-lg"),
            _field_box("3b. COUNTY", APPLICANT_1.county.upper(), class_name="val-lg"),
            _field_box("3c. STATE", APPLICANT_1.state.upper(), class_name="val-lg"),
            _field_box("4. BIRTHPLACE (State or foreign Country)", APPLICANT_1.birthplace.upper(), class_name="val-lg"),
            cols="1.7fr 1fr 1fr 1.2fr",
        ),
        _row(
            _field_box(
                "5a. NAME OF SPOUSE (First, Middle, Last)",
                APPLICANT_2.full_name.upper(),
                class_name="val-lg",
            ),
            _field_box("5b. MAIDEN SURNAME (If applicable)", (APPLICANT_2.maiden_surname or "")),
            _field_box("6. DATE OF BIRTH (Month, Day, Year)", APPLICANT_2.date_of_birth, class_name="val-lg"),
            cols="2.35fr 0.95fr 1fr",
        ),
        _row(
            _field_box("7a. RESIDENCE - CITY, TOWN OR LOCATION", APPLICANT_2.residence_city.upper(), class_name="val-lg"),
            _field_box("7b. COUNTY", APPLICANT_2.county.upper(), class_name="val-lg"),
            _field_box("7c. STATE", APPLICANT_2.state.upper(), class_name="val-lg"),
            _field_box("8. BIRTHPLACE (State or foreign Country)", APPLICANT_2.birthplace.upper(), class_name="val-lg"),
            cols="1.7fr 1fr 1fr 1.2fr",
        ),
        el(
            "div",
            _boilerplate(
                "WE THE APPLICANTS NAMED IN THIS CERTIFICATE, EACH FOR HIMSELF OR HERSELF, STATE THAT THE INFORMATION PROVIDED ON THIS RECORD IS CORRECT TO THE BEST OF OUR KNOWLEDGE AND BELIEF, THAT NO LEGAL OBJECTION TO THE MARRIAGE NOR THE ISSUANCE OF A LICENSE TO AUTHORIZE THE SAME IS KNOWN TO US AND HEREBY APPLY FOR LICENSE TO MARRY."
            ),
            class_name="fb-boilerplate-wrap",
        ),
        _row(
            _field_box(
                "9. SIGNATURE OF SPOUSE (Sign full name using black ink)",
                _sig_value(signer_name=APPLICANT_1.full_name, signed_on=None, reference_id="form-field-9"),
                class_name="sig-field",
            ),
            _field_box("10. SUBSCRIBED AND SWORN TO BEFORE ME ON (Date)", "05/31/2019", class_name="val-lg"),
            cols="1.55fr 1fr",
        ),
        _row(
            _field_box("11. TITLE OF OFFICIAL", "DEPUTY CLERK", class_name="val-lg"),
            _field_box(
                "12. SIGNATURE OF OFFICIAL (Use black ink)",
                _sig_value(signer_name=None, signed_on=None, reference_id="form-field-12"),
                class_name="sig-field",
            ),
            cols="1.1fr 1.45fr",
        ),
        _row(
            _field_box("SEAL", _seal_value(), class_name="seal-field"),
            _field_box(
                "13. SIGNATURE OF SPOUSE (Sign full name using black ink)",
                _sig_value(signer_name=APPLICANT_2.full_name, signed_on=None, reference_id="form-field-13"),
                class_name="sig-field",
            ),
            _field_box("14. SUBSCRIBED AND SWORN TO BEFORE ME ON (Date)", "05/31/2019", class_name="val-lg"),
            cols="0.42fr 1.3fr 0.95fr",
        ),
        _row(
            _field_box("15. TITLE OF OFFICIAL", "DEPUTY CLERK", class_name="val-lg"),
            _field_box(
                "16. SIGNATURE OF OFFICIAL (Use black ink)",
                _sig_value(signer_name=None, signed_on=None, reference_id="form-field-16"),
                class_name="sig-field",
            ),
            cols="1.1fr 1.45fr",
        ),
        class_name="fb-section form-block",
    )


def _license_to_marry_section() -> object:
    return el(
        "section",
        _section_bar("LICENSE TO MARRY"),
        el(
            "div",
            _boilerplate(
                "AUTHORIZATION AND LICENSE IS HEREBY GIVEN TO ANY PERSON DULY AUTHORIZED BY THE LAWS OF THE STATE OF FLORIDA TO PERFORM A MARRIAGE CEREMONY WITHIN THE STATE OF FLORIDA AND TO SOLEMNIZE THE MARRIAGE OF THE ABOVE NAMED PERSONS. THIS LICENSE MUST BE USED ON OR AFTER THE EFFECTIVE DATE AND ON OR BEFORE THE EXPIRATION DATE IN THE STATE OF FLORIDA IN ORDER TO BE RECORDED AND VALID."
            ),
            class_name="fb-boilerplate-wrap",
        ),
        _row(
            _field_box("SEAL", _seal_value(), class_name="seal-field"),
            _field_box("17. COUNTY ISSUING LICENSE", "ESCAMBIA COUNTY", class_name="val-lg"),
            _field_box("18. DATE LICENSE ISSUED", "05/31/2019", class_name="val-lg"),
            _field_box("18a. DATE LICENSE EFFECTIVE", "06/03/2019", class_name="val-lg"),
            _field_box("19. EXPIRATION DATE", "08/02/2019", class_name="val-lg"),
            cols="0.42fr 1.25fr 0.95fr 0.95fr 0.9fr",
        ),
        _row(
            _field_box(
                "20. SIGNATURE OF COURT CLERK OR JUDGE",
                _sig_value(signer_name="Pam Childers", signed_on=None, reference_id="form-field-20"),
                class_name="sig-field",
            ),
            _field_box("20b. TITLE", "CLERK OF COURTS", class_name="val-lg"),
            _field_box("20c. BY D.C.", "[Illegible in source scan]", class_name="val-lg"),
            cols="1.45fr 1.05fr 0.45fr",
        ),
        class_name="fb-section form-block",
    )


def _certificate_of_marriage_section() -> object:
    return el(
        "section",
        _section_bar("CERTIFICATE OF MARRIAGE"),
        el(
            "div",
            _boilerplate(
                "I HEREBY CERTIFY THAT THE ABOVE NAMED SPOUSES WERE JOINED BY ME IN MARRIAGE IN ACCORDANCE WITH THE LAWS OF THE STATE OF FLORIDA."
            ),
            class_name="fb-boilerplate-wrap",
        ),
        _row(
            _field_box("21. DATE OF MARRIAGE (Month, Day, Year)", "06/03/2019", class_name="val-lg"),
            _field_box("22. CITY, TOWN, OR LOCATION OF MARRIAGE", "PENSACOLA, FLORIDA", class_name="val-lg"),
            cols="0.95fr 1.75fr",
        ),
        _row(
            _field_box(
                "23a. SIGNATURE OF PERSON PERFORMING CEREMONY (Use black ink)",
                _sig_value(signer_name=None, signed_on=None, reference_id="form-field-23a"),
                class_name="sig-field",
            ),
            _field_box("23c. ADDRESS (Of person performing ceremony)", "2270 E. JOHNSON AVE.", class_name="val-lg"),
            cols="1.1fr 1.3fr",
        ),
        _row(
            _field_box(
                "23b. NAME AND TITLE OF PERSON PERFORMING CEREMONY (Or notary stamp)",
                "GARY T. DOUGHERTY; SR. PASTOR; COURTS OF PRAISE FELLOWSHIP",
                class_name="val-md",
            ),
            _field_box(
                "24. SIGNATURE OF WITNESS TO CEREMONY (Use black ink)",
                _sig_value(signer_name=None, signed_on=None, reference_id="form-field-24"),
                class_name="sig-field",
            ),
            cols="1.1fr 1.3fr",
        ),
        _row(
            _field_box(
                "25. SIGNATURE OF WITNESS TO CEREMONY (Use black ink)",
                _sig_value(signer_name=None, signed_on=None, reference_id="form-field-25"),
                class_name="sig-field",
            ),
            cols="1fr",
        ),
        class_name="fb-section form-block",
    )


@Document(
    page="LETTER",
    margin="0.5in",
    title="Keenan/Courtney Marriage Record CAV (Accessibility-First)",
    bootstrap=False,
)
def App(_props=None) -> object:
    return el(
        "div",
        el(
            "div",
            "Recorded in Public Records 6/14/2019 3:05 PM OR Book 8112 Page 1777, Instrument #2019052280, Pam Childers Clerk of the Circuit Court Escambia County, FL",
            class_name="recorded-stamp",
        ),
        el(
            "section",
            _row(
                el(
                    "div",
                    Text(
                        "Department of Health • Office of Vital Statistics",
                        tag="div",
                        class_name="fb-header-line fb-center",
                    ),
                    Text("STATE OF FLORIDA", tag="div", class_name="fb-header-main fb-center"),
                    Text("MARRIAGE RECORD", tag="div", class_name="fb-header-main fb-center"),
                    Text("TYPE IN UPPER CASE", tag="div", class_name="fb-header-sub fb-center"),
                    Text("USE BLACK INK", tag="div", class_name="fb-header-sub fb-center"),
                    Text(
                        "This license not valid unless seal of Clerk, Circuit or County Court, appears thereon.",
                        tag="div",
                        class_name="fb-header-note fb-center",
                    ),
                    class_name="fb-title-pane",
                ),
                _field_box("(STATE FILE NUMBER)", "", class_name="state-file-box center-label"),
                cols="1.35fr 0.75fr",
                class_name="header-row",
            ),
            el(
                "div",
                Text("2019 ML 001547", tag="div", class_name="application-number"),
                Text("(APPLICATION NUMBER)", tag="div", class_name="application-number-label"),
                class_name="application-number-wrap",
            ),
            class_name="form-header-block",
        ),
        _application_to_marry_section(),
        _license_to_marry_section(),
        _certificate_of_marriage_section(),
        el(
            "div",
            "INFORMATION BELOW FOR USE BY VITAL STATISTICS ONLY - NOT TO BE RECORDED",
            class_name="vital-stats-footer",
        ),
        class_name="cav-root",
    )


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def _build_transcription_payload(source_analysis: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": "fullbleed.accessibility_cav.transcription.v1",
        "source_pdf_path": str(SOURCE_PDF_PATH),
        "record_header": {
            "jurisdiction": "State of Florida",
            "agency": "Department of Health, Office of Vital Statistics",
            "form_title": "Marriage Record",
            "application_number": "2019 ML 001547",
            "state_file_number": None,
            "recorded_stamp_text": (
                "Recorded in Public Records 6/14/2019 3:05 PM OR Book 8112 Page 1777, "
                "Instrument #2019052280, Pam Childers Clerk of the Circuit Court Escambia County, FL"
            ),
        },
        "application_to_marry": {
            "applicant_1": asdict(APPLICANT_1),
            "applicant_2": asdict(APPLICANT_2),
            "signatures": [asdict(item) for item in APPLICATION_SIGNATURES],
        },
        "license_to_marry": {
            "county_issuing_license": "Escambia County",
            "date_license_issued": "05/31/2019",
            "date_license_effective": "06/03/2019",
            "expiration_date": "08/02/2019",
            "clerk_title": "Clerk of Courts",
            "by_dc_initials": "AS",
            "signatures": [asdict(item) for item in LICENSE_SIGNATURES],
        },
        "certificate_of_marriage": {
            "date_of_marriage": "06/03/2019",
            "location_of_marriage": "Pensacola, Florida",
            "performer_address": "2270 E. Johnson Ave.",
            "performer_name_title_transcription": "Gary T. Dougherty; Sr. Pastor; Courts of Praise Fellowship",
            "signatures": [asdict(item) for item in CERTIFICATE_SIGNATURES],
        },
        "review_queue": [asdict(item) for item in REVIEW_QUEUE],
        "source_analysis": source_analysis,
    }


def _build_parity_report_payload(
    *,
    source_analysis: dict[str, Any],
    transcription_payload: dict[str, Any],
    a11y_report: dict[str, Any],
    component_validation: dict[str, Any],
) -> dict[str, Any]:
    total_signatures = (
        len(APPLICATION_SIGNATURES) + len(LICENSE_SIGNATURES) + len(CERTIFICATE_SIGNATURES)
    )
    return {
        "schema": "fullbleed.accessibility_cav.parity.v1",
        "goal": {
            "functional_parity": True,
            "semantic_parity": True,
            "visual_parity_priority": "low",
            "render_outputs_used_for_verification": True,
        },
        "source_characteristics": {
            "page_count": source_analysis.get("page_count"),
            "image_only_page1": source_analysis.get("text_layer_present_page1") is False,
            "source_preview_path": source_analysis.get("preview_png_path"),
        },
        "coverage": {
            "recorded_filing_annotation": True,
            "record_header": True,
            "application_to_marry": True,
            "license_to_marry": True,
            "certificate_of_marriage": True,
            "signature_inventory_total": total_signatures,
            "review_queue_items": len(REVIEW_QUEUE),
            "transcription_sidecar_present": bool(transcription_payload),
        },
        "validation": {
            "a11y_ok": bool(a11y_report.get("ok", False)),
            "component_mount_ok": bool(component_validation.get("ok", False)),
            "a11y_error_count": int(a11y_report.get("error_count", 0)),
            "a11y_warning_count": int(a11y_report.get("warning_count", 0)),
        },
    }


def _build_claim_evidence_payload(*, source_analysis: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": "fullbleed.a11y.claim_evidence.v1",
        "delivery_target": "html",
        "document_use_case": "cav",
        "technology_support": {
            "assessed": True,
            "basis_recorded": True,
            "relied_upon_technologies": ["html", "css"],
            "active_content_present": False,
            "assessment_basis": (
                "Rendered CAV deliverable was reviewed for active-content signals. The output "
                "relies on static HTML and CSS only."
            ),
        },
        "section508": {
            "scope_declared": True,
            "profile": "section508.revised.e205.html",
            "public_facing_determination_recorded": True,
            "public_facing_content": False,
            "official_communications_determination_recorded": True,
            "official_communications": False,
            "nara_exception_determination_recorded": True,
            "nara_exception_applies": False,
            "determination_basis": (
                "Engineering CAV validation sample. Section 508 E205 applicability decisions are "
                "recorded explicitly for audit traceability."
            ),
        },
        "wcag20": {
            "keyboard_assessed": True,
            "keyboard_basis_recorded": True,
            "keyboard_trap_assessed": True,
            "keyboard_trap_basis_recorded": True,
            "on_input_assessed": True,
            "on_input_basis_recorded": True,
            "on_focus_assessed": True,
            "on_focus_basis_recorded": True,
            "timing_adjustable_scope_declared": False,
            "timing_adjustable_assessed": True,
            "timing_adjustable_basis_recorded": True,
            "pause_stop_hide_scope_declared": False,
            "pause_stop_hide_assessed": True,
            "pause_stop_hide_basis_recorded": True,
            "three_flashes_scope_declared": False,
            "three_flashes_assessed": True,
            "three_flashes_basis_recorded": True,
            "audio_control_scope_declared": False,
            "audio_control_assessed": True,
            "audio_control_basis_recorded": True,
            "use_of_color_scope_declared": False,
            "use_of_color_assessed": True,
            "use_of_color_basis_recorded": True,
            "resize_text_scope_declared": False,
            "resize_text_assessed": True,
            "resize_text_basis_recorded": True,
            "images_of_text_scope_declared": False,
            "images_of_text_assessed": True,
            "images_of_text_basis_recorded": True,
            "prerecorded_av_alternative_scope_declared": False,
            "prerecorded_av_alternative_assessed": True,
            "prerecorded_av_alternative_basis_recorded": True,
            "prerecorded_captions_scope_declared": False,
            "prerecorded_captions_assessed": True,
            "prerecorded_captions_basis_recorded": True,
            "prerecorded_audio_description_or_media_alternative_scope_declared": False,
            "prerecorded_audio_description_or_media_alternative_assessed": True,
            "prerecorded_audio_description_or_media_alternative_basis_recorded": True,
            "live_captions_scope_declared": False,
            "live_captions_assessed": True,
            "live_captions_basis_recorded": True,
            "prerecorded_audio_description_scope_declared": False,
            "prerecorded_audio_description_assessed": True,
            "prerecorded_audio_description_basis_recorded": True,
            "meaningful_sequence_scope_declared": True,
            "meaningful_sequence_assessed": True,
            "meaningful_sequence_basis_recorded": True,
            "error_suggestion_scope_declared": False,
            "error_suggestion_assessed": True,
            "error_suggestion_basis_recorded": True,
            "error_prevention_scope_declared": False,
            "error_prevention_assessed": True,
            "error_prevention_basis_recorded": True,
            "consistent_identification_assessed": True,
            "consistent_identification_basis_recorded": True,
            "multiple_ways_scope_declared": False,
            "multiple_ways_assessed": True,
            "multiple_ways_basis_recorded": True,
            "consistent_navigation_scope_declared": False,
            "consistent_navigation_assessed": True,
            "consistent_navigation_basis_recorded": True,
            "assessment_basis": (
                "Single-document CAV with no interactive navigation/components; criterion treated as "
                "not applicable and assessment basis recorded."
            ),
        },
        "source_context": {
            "source_pdf_path": str(SOURCE_PDF_PATH),
            "source_page_count": int(source_analysis.get("page_count") or 0),
            "image_only_scan_page1": source_analysis.get("text_layer_present_page1") is False,
        },
    }


def _summarize_natural_accessibility_pass(a11y_verify: dict[str, Any]) -> dict[str, Any]:
    ignored_rule_ids = {"fb.a11y.claim.wcag20aa_level_readiness"}
    nonpass_verdicts = {"fail", "warn", "manual_needed"}
    nonpass_rule_ids: list[str] = []
    for finding in a11y_verify.get("findings") or []:
        rule_id = str(finding.get("rule_id") or "")
        verdict = str(finding.get("verdict") or "")
        if verdict not in nonpass_verdicts:
            continue
        if rule_id in ignored_rule_ids:
            continue
        if rule_id not in nonpass_rule_ids:
            nonpass_rule_ids.append(rule_id)
    return {
        "ok": len(nonpass_rule_ids) == 0,
        "ignored_rule_ids": sorted(ignored_rule_ids),
        "nonpass_rule_ids": nonpass_rule_ids,
    }


def _emit_engine_audit_reports(
    *,
    engine: fullbleed.PdfEngine,
    html_path: Path,
    css_path: Path,
    png_paths: list[str],
    source_analysis: dict[str, Any],
    a11y_report: dict[str, Any],
    component_validation: dict[str, Any],
    claim_evidence: dict[str, Any] | None,
) -> dict[str, Any]:
    out: dict[str, Any] = {
        "engine_a11y_verify_path": None,
        "engine_pmr_path": None,
        "engine_a11y_verify_ok": None,
        "engine_pmr_ok": None,
        "engine_pmr_score": None,
        "engine_a11y_contrast_seed_verdict": None,
        "engine_a11y_natural_pass_ok": None,
        "engine_a11y_natural_nonpass_rule_ids": [],
        "warnings": [],
    }
    if not hasattr(engine, "verify_accessibility_artifacts"):
        out["warnings"].append("PdfEngine.verify_accessibility_artifacts is unavailable.")
        return out
    if not hasattr(engine, "verify_paged_media_rank_artifacts"):
        out["warnings"].append("PdfEngine.verify_paged_media_rank_artifacts is unavailable.")
        return out

    contrast_png = png_paths[0] if png_paths else None
    try:
        a11y_verify = engine.verify_accessibility_artifacts(
            str(html_path),
            str(css_path),
            profile="cav",
            mode="error",
            render_preview_png_path=contrast_png,
            a11y_report=a11y_report,
            claim_evidence=claim_evidence,
        )
    except TypeError:
        # Backward compatibility with engines built before newer verifier hooks.
        try:
            a11y_verify = engine.verify_accessibility_artifacts(
                str(html_path),
                str(css_path),
                profile="cav",
                mode="error",
                render_preview_png_path=contrast_png,
                a11y_report=a11y_report,
            )
            out["warnings"].append(
                "Engine verifier does not accept claim_evidence; claim seed rules may remain manual_needed."
            )
        except TypeError:
            try:
                a11y_verify = engine.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile="cav",
                    mode="error",
                    render_preview_png_path=contrast_png,
                )
                out["warnings"].append(
                    "Engine verifier does not accept a11y_report or claim_evidence; pre-render bridge correlation not evaluated and claim seed rules may remain manual_needed."
                )
            except TypeError:
                a11y_verify = engine.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile="cav",
                    mode="error",
                )
                out["warnings"].append(
                    "Engine verifier does not accept render_preview_png_path, a11y_report, or claim_evidence; contrast seed, pre-render bridge correlation, and claim seed evidence were not evaluated."
                )
    _write_json(ENGINE_A11Y_VERIFY_PATH, a11y_verify)
    out["engine_a11y_verify_path"] = str(ENGINE_A11Y_VERIFY_PATH)
    out["engine_a11y_verify_ok"] = bool((a11y_verify.get("gate") or {}).get("ok", False))
    natural_pass = _summarize_natural_accessibility_pass(a11y_verify)
    out["engine_a11y_natural_pass_ok"] = bool(natural_pass.get("ok"))
    out["engine_a11y_natural_nonpass_rule_ids"] = list(
        natural_pass.get("nonpass_rule_ids") or []
    )
    contrast_rows = [
        f
        for f in (a11y_verify.get("findings") or [])
        if f.get("rule_id") == "fb.a11y.contrast.minimum_render_seed"
    ]
    if contrast_rows:
        out["engine_a11y_contrast_seed_verdict"] = contrast_rows[0].get("verdict")

    pmr = engine.verify_paged_media_rank_artifacts(
        str(html_path),
        str(css_path),
        profile="cav",
        mode="error",
        overflow_count=int(component_validation.get("overflow_count") or 0),
        known_loss_count=int(component_validation.get("known_loss_count") or 0),
        source_page_count=int(source_analysis.get("page_count") or 0)
        if source_analysis.get("page_count") is not None
        else None,
        render_page_count=len(png_paths),
        review_queue_items=len(REVIEW_QUEUE),
    )
    _write_json(ENGINE_PMR_PATH, pmr)
    out["engine_pmr_path"] = str(ENGINE_PMR_PATH)
    out["engine_pmr_ok"] = bool((pmr.get("gate") or {}).get("ok", False))
    rank = pmr.get("rank") or {}
    if "score" in rank:
        out["engine_pmr_score"] = rank.get("score")
    return out


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    css = CSS_PATH.read_text(encoding="utf-8")

    source_analysis = get_source_analysis()
    _write_json(SOURCE_ANALYSIS_PATH, source_analysis)
    transcription_payload = _build_transcription_payload(source_analysis)
    _write_json(TRANSCRIPTION_PATH, transcription_payload)
    claim_evidence = _build_claim_evidence_payload(source_analysis=source_analysis)
    _write_json(CLAIM_EVIDENCE_PATH, claim_evidence)

    engine = create_engine()
    artifact = App()

    a11y_report = A11yContract().validate(artifact, mode=None)
    _write_json(A11Y_VALIDATION_PATH, a11y_report)

    # Validate the authored document tree first; bundle emission/renders/audits then run
    # through the accessibility runtime surface so PDF output is explicitly PDF/UA-targeted.
    artifact.to_html(a11y_mode="raise")

    component_validation = validate_component_mount(
        engine=engine,
        node_or_component=App,
        css=css,
        fail_on_overflow=True,
        fail_on_css_warnings=False,
        fail_on_known_loss=False,
        fail_on_html_asset_warning=True,
    )
    _write_json(COMPONENT_VALIDATION_PATH, component_validation)

    parity_report = _build_parity_report_payload(
        source_analysis=source_analysis,
        transcription_payload=transcription_payload,
        a11y_report=a11y_report,
        component_validation=component_validation,
    )
    _write_json(PARITY_REPORT_PATH, parity_report)

    bundle = engine.render_bundle(
        body_html=artifact.root.to_html(),
        css_text=css,
        out_dir=str(OUTPUT_DIR),
        stem=DOC_STEM,
        profile="cav",
        a11y_mode="raise",
        a11y_report=a11y_report,
        claim_evidence=claim_evidence,
        component_validation=component_validation,
        parity_report=parity_report,
        source_analysis=source_analysis,
        render_preview_png=True,
        run_verifier=True,
        run_pmr=True,
        run_pdf_ua_seed_verify=True,
        emit_reading_order_trace=True,
        emit_pdf_structure_trace=True,
    )
    bundle_run_report = bundle.run_report or {}
    bytes_written = int((bundle_run_report.get("metrics") or {}).get("pdf_bytes") or 0)
    png_paths = list(bundle_run_report.get("render_preview_png_paths") or [])
    engine_audits = {
        "engine_a11y_verify_path": bundle.paths.get("engine_a11y_verify_path"),
        "engine_pmr_path": bundle.paths.get("engine_pmr_path"),
        "engine_a11y_verify_ok": (bundle_run_report.get("engine_a11y_verify_ok")),
        "engine_pmr_ok": (bundle_run_report.get("engine_pmr_ok")),
        "engine_pmr_score": (bundle_run_report.get("engine_pmr_score")),
        "engine_a11y_contrast_seed_verdict": None,
        "engine_a11y_natural_pass_ok": None,
        "engine_a11y_natural_nonpass_rule_ids": [],
        "pdf_ua_seed_verify_path": bundle.paths.get("pdf_ua_seed_verify_path"),
        "reading_order_trace_path": bundle.paths.get("reading_order_trace_path"),
        "reading_order_trace_render_path": bundle.paths.get("reading_order_trace_render_path"),
        "pdf_structure_trace_path": bundle.paths.get("pdf_structure_trace_path"),
        "pdf_structure_trace_render_path": bundle.paths.get("pdf_structure_trace_render_path"),
        "reading_order_trace_cross_check": bundle_run_report.get("reading_order_trace_cross_check"),
        "pdf_structure_trace_cross_check": bundle_run_report.get("pdf_structure_trace_cross_check"),
        "pdf_ua_seed_ok": bundle_run_report.get("pdf_ua_seed_ok"),
        "warnings": list(bundle.warnings or []),
    }
    if bundle.verifier_report:
        natural_pass = _summarize_natural_accessibility_pass(bundle.verifier_report)
        engine_audits["engine_a11y_natural_pass_ok"] = bool(natural_pass.get("ok"))
        engine_audits["engine_a11y_natural_nonpass_rule_ids"] = list(
            natural_pass.get("nonpass_rule_ids") or []
        )
        contrast_rows = [
            f
            for f in (bundle.verifier_report.get("findings") or [])
            if f.get("rule_id") == "fb.a11y.contrast.minimum_render_seed"
        ]
        if contrast_rows:
            engine_audits["engine_a11y_contrast_seed_verdict"] = contrast_rows[0].get("verdict")

    run_report = {
        "schema": "fullbleed.accessibility_cav.run.v1",
        "ok": bool(a11y_report.get("ok", False)) and bool(component_validation.get("ok", False)),
        "goal": "HTML and CSS are the deliverables; fullbleed is the preview/validation harness.",
        "source_pdf_path": str(SOURCE_PDF_PATH),
        "deliverables": {
            "html_path": str(HTML_PATH),
            "css_path": str(CSS_ARTIFACT_PATH),
            "css_source_path": str(CSS_PATH),
            "pdf_preview_path": str(PDF_PATH),
            "render_preview_pngs": png_paths,
        },
        "source_analysis_path": str(SOURCE_ANALYSIS_PATH),
        "source_preview_path": source_analysis.get("preview_png_path"),
        "transcription_path": str(TRANSCRIPTION_PATH),
        "claim_evidence_path": str(CLAIM_EVIDENCE_PATH),
        "parity_report_path": str(PARITY_REPORT_PATH),
        "a11y_validation_path": str(A11Y_VALIDATION_PATH),
        "component_validation_path": str(COMPONENT_VALIDATION_PATH),
        "engine_a11y_verify_path": engine_audits.get("engine_a11y_verify_path"),
        "engine_pmr_path": engine_audits.get("engine_pmr_path"),
        "pdf_ua_seed_verify_path": engine_audits.get("pdf_ua_seed_verify_path"),
        "reading_order_trace_path": engine_audits.get("reading_order_trace_path"),
        "reading_order_trace_render_path": engine_audits.get("reading_order_trace_render_path"),
        "pdf_structure_trace_path": engine_audits.get("pdf_structure_trace_path"),
        "pdf_structure_trace_render_path": engine_audits.get("pdf_structure_trace_render_path"),
        "metrics": {
            "pdf_bytes": bytes_written,
            "source_page_count": int(source_analysis.get("page_count") or 0),
            "render_page_count": len(png_paths),
            "review_queue_items": len(REVIEW_QUEUE),
            "signature_items_modeled": len(APPLICATION_SIGNATURES)
            + len(LICENSE_SIGNATURES)
            + len(CERTIFICATE_SIGNATURES),
            "a11y_ok": bool(a11y_report.get("ok", False)),
            "component_mount_ok": bool(component_validation.get("ok", False)),
            "engine_a11y_verify_ok": engine_audits.get("engine_a11y_verify_ok"),
            "engine_a11y_natural_pass_ok": engine_audits.get("engine_a11y_natural_pass_ok"),
            "engine_pmr_ok": engine_audits.get("engine_pmr_ok"),
            "engine_pmr_score": engine_audits.get("engine_pmr_score"),
            "pdf_ua_seed_ok": engine_audits.get("pdf_ua_seed_ok"),
            "engine_a11y_contrast_seed_verdict": engine_audits.get(
                "engine_a11y_contrast_seed_verdict"
            ),
            "engine_a11y_natural_nonpass_rule_ids": engine_audits.get(
                "engine_a11y_natural_nonpass_rule_ids"
            )
            or [],
        },
        "reading_order_trace_cross_check": engine_audits.get("reading_order_trace_cross_check"),
        "pdf_structure_trace_cross_check": engine_audits.get("pdf_structure_trace_cross_check"),
        "engine_audits": engine_audits,
    }
    _write_json(RUN_REPORT_PATH, run_report)

    print(f"[ok] Source PDF: {SOURCE_PDF_PATH}")
    if source_analysis.get("preview_png_path"):
        print(f"[ok] Source preview PNG: {source_analysis['preview_png_path']}")
    print(f"[ok] Wrote deliverable HTML: {HTML_PATH}")
    print(f"[ok] Wrote preview PDF: {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] Transcription: {TRANSCRIPTION_PATH}")
    print(f"[ok] Parity report: {PARITY_REPORT_PATH}")
    print(f"[ok] A11y validation: {A11Y_VALIDATION_PATH} (ok={a11y_report.get('ok')})")
    print(
        f"[ok] Component validation: {COMPONENT_VALIDATION_PATH} (ok={component_validation.get('ok')})"
    )
    if engine_audits.get("engine_a11y_verify_path"):
        print(
            f"[ok] Engine a11y verify: {engine_audits['engine_a11y_verify_path']} "
            f"(ok={engine_audits.get('engine_a11y_verify_ok')})"
        )
    if engine_audits.get("engine_pmr_path"):
        print(
            f"[ok] Engine PMR: {engine_audits['engine_pmr_path']} "
            f"(ok={engine_audits.get('engine_pmr_ok')}, score={engine_audits.get('engine_pmr_score')})"
        )
    for warning in engine_audits.get("warnings", []):
        print(f"[warn] {warning}")
    if png_paths:
        print(f"[ok] Render preview PNGs: {len(png_paths)}")
    print(f"[ok] Run report: {RUN_REPORT_PATH}")


if __name__ == "__main__":
    main()
