# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import FieldGrid, FieldItem, Heading, Region, Section
from ..primitives import Box, LayoutGrid, Stack, Text
from ._core import CavKitBase, CavProfile


AGENCY_LETTER_FAMILY_ID = "agency_letter_cav"


FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_NOTICE_V1 = CavProfile(
    profile_id="fl.escambia.agency_notice_letter.single_page_clerk_letterhead_notice.v1",
    profile_version=1,
    family_id=AGENCY_LETTER_FAMILY_ID,
    revision="single_page_clerk_letterhead_notice_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Clerk of the Circuit Court and Comptroller",
    display_name="Escambia Agency Notice Letter - Single Page Clerk Letterhead Notice V1",
    supported_variants=(
        "single-page county notice letters with clerk letterhead, date/subject lines, prose body, and footer contact lines",
    ),
    coverage_notes=(
        "Profile is layout-scoped for county/agency notice letters using the Escambia Clerk letterhead layout represented by the TDT increase letter and jury-scam warning letter exemplars.",
        "Body prose and links are represented as structured text/anchors; decorative seal/logo imagery may be represented textually in v1.",
    ),
    unsupported_features=(
        "exact logo artwork reproduction in v1",
        "signature-image reproduction when source contains only typed signoff in v1",
    ),
)


FL_ESCAMBIA_AGENCY_NOTICE_PUBLIC_NOTICE_VAB_RESCHEDULED_2020_V1 = CavProfile(
    profile_id="fl.escambia.agency_notice.public_notice_vab_rescheduled_2020.v1",
    profile_version=1,
    family_id=AGENCY_LETTER_FAMILY_ID,
    revision="public_notice_vab_rescheduled_2020_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Value Adjustment Board / Escambia Clerk",
    display_name="Escambia Public Notice - VAB Rescheduled Meeting (January 2020) V1",
    supported_variants=(
        "single-page public notice text blocks with centered heading, dated line, and no signature block",
    ),
    coverage_notes=(
        "Profile is layout-scoped to the one-page public notice document sourced from Escambia DocCenter item 742.",
        "Notice body is represented as text-first content preserving meeting date/time/location and dated line semantics.",
    ),
    unsupported_features=(
        "publisher-specific print production metadata beyond visible notice content in v1",
    ),
)

# Transitional alias retained for early local exemplars that used an over-specific profile name.
FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_2021_V1 = (
    FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_NOTICE_V1
)


def _paragraph_nodes(items: Iterable[Any]) -> list[Any]:
    out: list[Any] = []
    for item in items:
        if isinstance(item, Mapping):
            segments = list(item.get("segments") or [])
            class_name = str(item.get("class_name") or "letter-body")
            children: list[Any] = []
            for seg in segments:
                if isinstance(seg, Mapping) and str(seg.get("href") or "").strip():
                    href = str(seg.get("href") or "").strip()
                    text = str(seg.get("text") or href)
                    children.append(el("a", text, href=href, class_name="letter-link"))
                else:
                    children.append(str(seg.get("text") if isinstance(seg, Mapping) else seg))
            out.append(el("p", *children, class_name=class_name))
            continue
        out.append(Text(str(item), tag="p", class_name="letter-body"))
    return out


def _resource_link_rows(items: Iterable[Mapping[str, Any]]) -> list[Any]:
    rows: list[Any] = []
    for item in items:
        label = str(item.get("label") or "Resource")
        href = str(item.get("href") or "").strip()
        if href:
            value = el("a", str(item.get("text") or href), href=href, class_name="letter-link")
        else:
            value = str(item.get("text") or "[Link not transcribed]")
        rows.append(FieldItem(label, value))
    return rows


@dataclass
class AgencyLetterCavKit(CavKitBase):
    family_id: str = AGENCY_LETTER_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "title",
        "document_heading",
        "show_letterhead",
        "letterhead",
        "date_line",
        "subject_line",
        "salutation",
        "paragraphs",
        "resource_links",
        "closing_block",
        "footer_lines",
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
        return _agency_letter_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.5in",
    title="Agency Notice Letter CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _agency_letter_document(payload: dict[str, Any]) -> object:
    header = dict(payload.get("letterhead") or {})
    closing = dict(payload.get("closing_block") or {})
    footer_lines = [str(x) for x in (payload.get("footer_lines") or []) if str(x).strip()]
    paragraphs = _paragraph_nodes(payload.get("paragraphs") or [])
    show_letterhead = bool(payload.get("show_letterhead", True))
    has_closing = any(
        str(closing.get(k) or "").strip() for k in ("closing", "signer_name", "signer_title")
    )

    signer_semantics = el(
        "span",
        str(closing.get("signer_name") or ""),
        data_fb_a11y_signature_status=str(closing.get("signature_status") or "not_required"),
        data_fb_a11y_signature_method=str(closing.get("signature_method") or "unknown"),
        data_fb_a11y_signature_ref=str(closing.get("signature_ref") or "agency-letter-signoff"),
        class_name="letter-signer-semantics",
    )

    date_line_value = str(payload.get("date_line") or "").strip()
    subject_line_value = str(payload.get("subject_line") or "").strip()
    salutation_value = str(payload.get("salutation") or "").strip()
    heading_value = str(payload.get("document_heading") or payload.get("title") or "").strip()
    letter_meta_items: list[Any] = []
    if date_line_value:
        letter_meta_items.append(FieldItem("Date", date_line_value))
    if subject_line_value:
        letter_meta_items.append(FieldItem("Subject", subject_line_value))

    return Stack(
        (
            Box(
                LayoutGrid(
                    Box(
                        Text(str(header.get("seal_text") or "Official seal emblem present"), tag="p", class_name="letter-seal"),
                        class_name="letter-seal-box",
                    ),
                    Box(
                        Heading(str(header.get("agency_name") or "Agency Name"), level=1),
                        Text(str(header.get("office_name") or ""), tag="p", class_name="letter-office-line"),
                        Text(str(header.get("suboffice_line") or ""), tag="p", class_name="letter-suboffice-line"),
                        class_name="letterhead-text-box",
                    ),
                    class_name="letterhead-grid",
                ),
                class_name="letterhead-card",
            )
            if show_letterhead
            else (
                Box(
                    Heading(heading_value, level=1),
                    class_name="letter-heading-box",
                )
                if heading_value
                else None
            )
        ),
        (
            Region(
                FieldGrid(*letter_meta_items),
                label="Letter metadata",
                class_name="letter-meta-region",
            )
            if letter_meta_items
            else None
        ),
        Region(
            (Text(salutation_value, tag="p", class_name="letter-salutation") if salutation_value else None),
            *paragraphs,
            label="Letter body",
            class_name="letter-body-section",
        ),
        (
            Section(
                Heading("Referenced Links", level=2),
                Region(
                    FieldGrid(*_resource_link_rows(payload.get("resource_links") or [])),
                    label="Letter hyperlinks",
                    class_name="letter-links-region",
                ),
                class_name="letter-links-section",
            )
            if list(payload.get("resource_links") or [])
            else None
        ),
        (
            Box(
                Text(str(closing.get("closing") or "Sincerely,"), tag="p", class_name="letter-closing-line"),
                el("p", signer_semantics, class_name="letter-signer-line"),
                Text(str(closing.get("signer_title") or ""), tag="p", class_name="letter-signer-title"),
                class_name="letter-closing-box",
            )
            if has_closing
            else None
        ),
        (
            Box(
                *[Text(line, tag="p", class_name="letter-footer-line") for line in footer_lines],
                class_name="letter-footer-box",
            )
            if footer_lines
            else None
        ),
        class_name="agency-letter-root",
    )


__all__ = [
    "AGENCY_LETTER_FAMILY_ID",
    "FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_NOTICE_V1",
    "FL_ESCAMBIA_AGENCY_NOTICE_PUBLIC_NOTICE_VAB_RESCHEDULED_2020_V1",
    "FL_ESCAMBIA_AGENCY_NOTICE_LETTER_SINGLE_PAGE_CLERK_LETTERHEAD_2021_V1",
    "AgencyLetterCavKit",
]
