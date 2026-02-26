# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import (
    FieldGrid,
    FieldItem,
    Heading,
    Region,
    Section,
)
from ..primitives import Box, LayoutGrid, Stack, Text
from ._core import CavKitBase, CavProfile


DECLARATION_FORM_FAMILY_ID = "declaration_form_cav"


FL_ESCAMBIA_DECLARATION_FORM_CDC_EVICTION_2020_LAYOUT_V1 = CavProfile(
    profile_id="fl.escambia.declaration_form.cdc_eviction_2020_layout.v1",
    profile_version=1,
    family_id=DECLARATION_FORM_FAMILY_ID,
    revision="cdc_eviction_2020_layout_v1",
    jurisdiction="US",
    county="Escambia",
    issuing_authority="CDC / public-use declaration copy (Escambia-sourced scan)",
    display_name="Declaration Form - CDC Eviction Declaration 2020 Layout V1",
    supported_variants=(
        "multi-page declaration forms with instructional prose, response lines, initials lines, and signature/contact block",
    ),
    coverage_notes=(
        "Profile is layout-scoped to the CDC temporary halt in evictions declaration form scan fetched from Escambia DocCenter item 3793.",
        "Text is transcribed/structured; response-line fields may remain blank in the CAV when blank on source.",
    ),
    unsupported_features=(
        "full legal provenance/official version verification beyond visible scanned content in v1",
    ),
)


def _response_line_block(prompt: str, *, lines: int = 3) -> object:
    line_placeholders = [Text("______________________________", tag="p", class_name="decl-line") for _ in range(max(1, int(lines)))]
    return Box(
        Text(prompt, tag="p", class_name="decl-prompt"),
        *line_placeholders,
        class_name="decl-response-block",
    )


def _statement_blocks(items: Iterable[Mapping[str, Any]]) -> list[Any]:
    out: list[Any] = []
    for item in items:
        prompt = str(item.get("prompt") or "[Statement not transcribed]")
        line_count = int(item.get("response_line_count") or 0)
        if line_count > 0:
            out.append(_response_line_block(prompt, lines=line_count))
        else:
            out.append(Text(prompt, tag="p", class_name="decl-body"))
    return out


def _initials_lines(items: Iterable[Mapping[str, Any]]) -> list[Any]:
    out: list[Any] = []
    for item in items:
        text = str(item.get("text") or "[Initials statement not transcribed]")
        out.append(
            Box(
                Text(text, tag="p", class_name="decl-body"),
                Text("(Please initial): __________", tag="p", class_name="decl-initial-line"),
                class_name="decl-initial-block",
            )
        )
    return out


def _slice_statement_blocks(items: list[Mapping[str, Any]], start: int, end: int | None = None) -> list[Any]:
    return _statement_blocks(items[start:end])


def _slice_initials_blocks(items: list[Mapping[str, Any]], start: int, end: int | None = None) -> list[Any]:
    return _initials_lines(items[start:end])


@dataclass
class DeclarationFormCavKit(CavKitBase):
    family_id: str = DECLARATION_FORM_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "title",
        "subtitle_lines",
        "intro_paragraphs",
        "declaration_lead",
        "statement_blocks",
        "initials_blocks",
        "signature_block",
        "authority_footer",
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
        return _declaration_form_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.35in",
    title="Declaration Form CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _declaration_form_document(payload: dict[str, Any]) -> object:
    sig = dict(payload.get("signature_block") or {})
    statement_items = [dict(x) for x in (payload.get("statement_blocks") or [])]
    initials_items = [dict(x) for x in (payload.get("initials_blocks") or [])]
    subtitle_lines = [str(x) for x in (payload.get("subtitle_lines") or []) if str(x).strip()]

    sig_text = el(
        "span",
        str(sig.get("signature_text") or "Signature line present"),
        data_fb_a11y_signature_status=str(sig.get("signature_status") or "unknown"),
        data_fb_a11y_signature_method=str(sig.get("signature_method") or "unknown"),
        data_fb_a11y_signature_ref=str(sig.get("signature_ref") or "declarant-signature"),
        class_name="decl-signature-semantics",
    )

    page_one = Region(
        Box(
            Heading(str(payload.get("title") or "Declaration Form"), level=1),
            *[Text(line, tag="p", class_name="decl-subtitle") for line in subtitle_lines],
            class_name="decl-title-box",
        ),
        Region(
            *[Text(str(p), tag="p", class_name="decl-body") for p in (payload.get("intro_paragraphs") or [])],
            label="Introductory declaration text",
            class_name="decl-section",
        ),
        Section(
            Heading("Declarant Statements", level=2),
            Text(str(payload.get("declaration_lead") or ""), tag="p", class_name="decl-lead"),
            *_slice_statement_blocks(statement_items, 0, 1),
            class_name="decl-section",
        ),
        label="Declaration form page 1",
        class_name="decl-page decl-page-1",
    )

    page_two = Region(
        Section(
            Heading("Declarant Statements (continued)", level=2),
            *_slice_statement_blocks(statement_items, 1, 4),
            class_name="decl-section",
        ),
        label="Declaration form page 2",
        class_name="decl-page decl-page-2 page-break-before",
    )

    page_three = Region(
        Section(
            Heading("Declarant Statements (continued)", level=2),
            *_slice_statement_blocks(statement_items, 4, None),
            *_slice_initials_blocks(initials_items, 0, None),
            class_name="decl-section",
        ),
        Section(
            Heading("Declarant Signature and Contact", level=2),
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem("Signature of Declarant", sig_text),
                        FieldItem("Date", sig.get("date") or "[Blank on form]"),
                        FieldItem("Print Name", sig.get("print_name") or "[Blank on form]"),
                        FieldItem("Phone #", sig.get("phone") or "[Blank on form]"),
                        FieldItem("Email", sig.get("email") or "[Blank on form]"),
                        FieldItem("Address", sig.get("address") or "[Blank on form]"),
                    ),
                    class_name="decl-card",
                ),
                class_name="decl-grid",
            ),
            class_name="decl-section",
        ),
        Box(
            Heading("Authority", level=3),
            *[Text(str(p), tag="p", class_name="decl-body") for p in (payload.get("authority_footer") or [])],
            class_name="decl-footer-box",
        ),
        label="Declaration form page 3",
        class_name="decl-page decl-page-3 page-break-before",
    )

    return Stack(page_one, page_two, page_three, class_name="decl-root")


__all__ = [
    "DECLARATION_FORM_FAMILY_ID",
    "FL_ESCAMBIA_DECLARATION_FORM_CDC_EVICTION_2020_LAYOUT_V1",
    "DeclarationFormCavKit",
]
