# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import FieldGrid, FieldItem, Heading, Region, Section
from ..primitives import Box, Stack, Text
from ._core import CavKitBase, CavProfile


INSTRUCTION_SHEET_FAMILY_ID = "instruction_sheet_cav"


FL_ESCAMBIA_INSTRUCTION_SHEET_CHILD_SUPPORT_PHONE_TESTIMONY_2019_V1 = CavProfile(
    profile_id="fl.escambia.instruction_sheet.child_support_phone_testimony_2019.v1",
    profile_version=1,
    family_id=INSTRUCTION_SHEET_FAMILY_ID,
    revision="child_support_phone_testimony_2019_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Family Law Division / Child Support Hearing Officer",
    display_name="Escambia Instruction Sheet - Child Support Telephone Testimony Instructions 2019 V1",
    supported_variants=(
        "single-page instruction sheets with title block, division line, numbered sections, and list-style conditions/instructions",
    ),
    coverage_notes=(
        "Profile is scoped to the Escambia Family Law Division child support telephone participation/testimony instructions form updated 6/2019.",
        "Instruction sections and list items are represented as structured headings, paragraphs, and ordered lists.",
    ),
    unsupported_features=(
        "exact line wraps or typographic spacing from the source PDF in v1",
    ),
)


FL_STATE_FAMILY_LAW_INSTRUCTION_PACKET_FORM_12_961_2018_V1 = CavProfile(
    profile_id="fl.state.family_law_instruction_packet.form12_961_notice_hearing_contempt_support_2018.v1",
    profile_version=1,
    family_id=INSTRUCTION_SHEET_FAMILY_ID,
    revision="family_law_form_12_961_09_2018_instruction_packet_v1",
    jurisdiction="FL",
    county=None,
    issuing_authority="Florida Supreme Court Approved Family Law Forms (distributed by county clerk portals)",
    display_name="Florida Family Law Instruction Packet - Form 12.961 Notice of Hearing on Motion for Contempt/Enforcement (09/18) V1",
    supported_variants=(
        "multi-page instruction packets with running header text, instruction prose pages, and appended form pages",
    ),
    coverage_notes=(
        "Profile is scoped to the Florida Supreme Court Approved Family Law Form 12.961 instruction packet (09/18) as distributed through Escambia Clerk DocCenter.",
        "Page parity is preserved using explicit page-grouped sections and authored page breaks.",
    ),
    unsupported_features=(
        "exact form-line spacing/kerning on the appended blank form pages in v1",
    ),
)

FL_STATE_FAMILY_LAW_INSTRUCTION_PACKET_FORM_12_921_2018_V1 = CavProfile(
    profile_id="fl.state.family_law_instruction_packet.form12_921_notice_hearing_child_support_enforcement_2018.v1",
    profile_version=1,
    family_id=INSTRUCTION_SHEET_FAMILY_ID,
    revision="family_law_form_12_921_06_2018_instruction_packet_v1",
    jurisdiction="FL",
    county=None,
    issuing_authority="Florida Supreme Court Approved Family Law Forms (distributed by county clerk portals)",
    display_name="Florida Family Law Instruction Packet - Form 12.921 Notice of Hearing (Child Support Enforcement Hearing Officer) (06/18) V1",
    supported_variants=(
        "multi-page instruction packets with running header text, instruction prose pages, and appended form pages",
    ),
    coverage_notes=(
        "Profile is scoped to the Florida Supreme Court Approved Family Law Form 12.921 instruction packet (06/18) as distributed through Escambia Clerk DocCenter.",
        "Page parity is preserved using explicit page-grouped sections and authored page breaks.",
    ),
    unsupported_features=(
        "exact form-line spacing/kerning on the appended blank form pages in v1",
    ),
)


def _ordered_items(items: Iterable[str], *, item_class: str = "instruction-list-item") -> Any | None:
    rows = [el("li", str(item), class_name=item_class) for item in items if str(item).strip()]
    if not rows:
        return None
    return el("ol", *rows, class_name="instruction-list")


def _paragraph_block(entry: Any, *, default_class: str = "instruction-body") -> Any:
    if isinstance(entry, Mapping):
        kind = str(entry.get("kind") or "").strip()
        text = str(entry.get("text") or "").strip()
        if not text:
            text = "[Blank line]"
        class_name = str(entry.get("class_name") or default_class).strip() or default_class
        attrs = dict(entry.get("attrs") or {})
        if kind == "signature_semantic_line":
            attrs.setdefault("data_fb_a11y_signature_status", str(entry.get("signature_status") or "unknown"))
            attrs.setdefault("data_fb_a11y_signature_method", str(entry.get("signature_method") or "signature_line_only"))
            if str(entry.get("signature_ref") or "").strip():
                attrs.setdefault("data_fb_a11y_signature_ref", str(entry.get("signature_ref")))
        return Text(text, tag="p", class_name=class_name, **attrs)
    return Text(str(entry), tag="p", class_name=default_class)


def _instruction_section(section: Mapping[str, Any], *, fallback_level: int = 2) -> Any:
    heading = str(section.get("heading") or "").strip()
    lead_paragraphs = [p for p in (section.get("paragraphs") or []) if str(p).strip()]
    items = [str(i) for i in (section.get("items") or []) if str(i).strip()]
    after_paragraphs = [p for p in (section.get("after_items_paragraphs") or []) if str(p).strip()]
    list_label = str(section.get("list_label") or "").strip()
    children: list[Any] = []
    if heading:
        children.append(Heading(heading, level=int(section.get("heading_level") or fallback_level)))
    children.extend(_paragraph_block(p, default_class="instruction-body") for p in lead_paragraphs)
    if list_label:
        children.append(Text(list_label, tag="p", class_name="instruction-list-label"))
    ol = _ordered_items(items)
    if ol is not None:
        children.append(ol)
    children.extend(_paragraph_block(p, default_class="instruction-body") for p in after_paragraphs)
    if not children:
        children.append(Text("[Section not transcribed]", tag="p", class_name="instruction-body"))
    return Section(*children, class_name="instruction-section")


def _instruction_page(page: Mapping[str, Any], *, page_index: int) -> Any:
    title_lines = [str(x) for x in (page.get("title_lines") or []) if str(x).strip()]
    metadata_fields = list(page.get("metadata_fields") or [])
    sections = list(page.get("sections") or [])
    running_header = str(page.get("running_header") or "").strip()
    page_label = str(page.get("page_label") or f"Instruction packet page {page_index + 1}")
    header_note = str(page.get("header_note") or "").strip()
    division_line = str(page.get("division_line") or "").strip()

    page_children: list[Any] = []
    if running_header:
        page_children.append(Text(running_header, tag="p", class_name="instruction-running-header"))

    if header_note or title_lines or division_line:
        page_children.append(
            Box(
                *([Text(header_note, tag="p", class_name="instruction-header-note")] if header_note else []),
                *[Heading(line, level=1 if idx == 0 else 2) for idx, line in enumerate(title_lines)],
                *([Text(division_line, tag="p", class_name="instruction-division-line")] if division_line else []),
                class_name="instruction-title-box instruction-title-box--page",
            )
        )

    if metadata_fields:
        page_children.append(
            Region(
                FieldGrid(
                    *[FieldItem(str(x.get("label") or "Field"), str(x.get("value") or "")) for x in metadata_fields]
                ),
                label=f"{page_label} metadata",
                class_name="instruction-meta-region",
            )
        )

    if sections:
        page_children.extend(_instruction_section(s) for s in sections)
    else:
        page_children.append(
            Section(
                Text("[Page content not transcribed]", tag="p", class_name="instruction-body"),
                class_name="instruction-section",
            )
        )

    if str(page.get("footer_note") or "").strip():
        page_children.append(Text(str(page.get("footer_note")), tag="p", class_name="instruction-page-footer-note"))

    page_classes = ["instruction-page"]
    page_style: dict[str, str] | None = None
    if page_index > 0:
        page_classes.append("instruction-page-break")
        page_style = {"break-before": "page", "page-break-before": "always"}
    return Region(
        *page_children,
        label=page_label,
        class_name=" ".join(page_classes),
        **({"style": page_style} if page_style else {}),
    )


@dataclass
class InstructionSheetCavKit(CavKitBase):
    family_id: str = INSTRUCTION_SHEET_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "header_note",
        "title_lines",
        "division_line",
        "metadata_fields",
        "sections",
        "pages",
        "running_header",
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
        return _instruction_sheet_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.45in",
    title="Instruction Sheet CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _instruction_sheet_document(payload: dict[str, Any]) -> object:
    title_lines = [str(x) for x in (payload.get("title_lines") or []) if str(x).strip()]
    metadata_fields = list(payload.get("metadata_fields") or [])
    sections = list(payload.get("sections") or [])
    pages = list(payload.get("pages") or [])
    running_header = str(payload.get("running_header") or "").strip()

    meta_grid = None
    if metadata_fields:
        meta_grid = Region(
            FieldGrid(*[FieldItem(str(x.get("label") or "Field"), str(x.get("value") or "")) for x in metadata_fields]),
            label="Instruction sheet metadata",
            class_name="instruction-meta-region",
        )

    if pages:
        normalized_pages: list[dict[str, Any]] = []
        for idx, p in enumerate(pages):
            page_map = dict(p)
            if idx == 0:
                page_map.setdefault("header_note", str(payload.get("header_note") or ""))
                page_map.setdefault("title_lines", title_lines)
                page_map.setdefault("division_line", str(payload.get("division_line") or ""))
                if metadata_fields and not page_map.get("metadata_fields"):
                    page_map["metadata_fields"] = metadata_fields
            if running_header and idx > 0:
                page_map.setdefault("running_header", running_header)
            normalized_pages.append(page_map)
        return Stack(*[_instruction_page(p, page_index=i) for i, p in enumerate(normalized_pages)], class_name="instruction-sheet-root")

    return Stack(
        Box(
            *(
                [Text(str(payload.get("header_note")), tag="p", class_name="instruction-header-note")]
                if str(payload.get("header_note") or "").strip()
                else []
            ),
            *[Heading(line, level=1 if idx == 0 else 2) for idx, line in enumerate(title_lines)],
            *(
                [Text(str(payload.get("division_line")), tag="p", class_name="instruction-division-line")]
                if str(payload.get("division_line") or "").strip()
                else []
            ),
            class_name="instruction-title-box",
        ),
        meta_grid,
        *[_instruction_section(s) for s in sections],
        class_name="instruction-sheet-root",
    )


__all__ = [
    "INSTRUCTION_SHEET_FAMILY_ID",
    "FL_ESCAMBIA_INSTRUCTION_SHEET_CHILD_SUPPORT_PHONE_TESTIMONY_2019_V1",
    "FL_STATE_FAMILY_LAW_INSTRUCTION_PACKET_FORM_12_961_2018_V1",
    "FL_STATE_FAMILY_LAW_INSTRUCTION_PACKET_FORM_12_921_2018_V1",
    "InstructionSheetCavKit",
]
