# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document, el
from ..accessibility import (
    ColumnHeader,
    DataCell,
    FigCaption,
    FieldGrid,
    FieldItem,
    Figure,
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


RECORDED_PLAT_FAMILY_ID = "recorded_plat_cav"


FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1 = CavProfile(
    profile_id="fl.escambia.recorded_plat.legacy_side_certificate_layout.v1",
    profile_version=1,
    family_id=RECORDED_PLAT_FAMILY_ID,
    revision="legacy_side_certificate_layout_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Clerk / Recorder",
    display_name="Escambia Recorded Plat - Legacy Side-Certificate Layout V1",
    supported_variants=(
        "single-sheet recorded plats with large map pane and side certificate/dedication column",
        "legacy PB1 exemplars (e.g., PB 1 PG 85 / 87 / 89)",
    ),
    coverage_notes=(
        "County-scoped layout-class profile for legacy Escambia recorded plat pages with side certificate blocks.",
        "Map image is preserved as an informative figure; textual metadata/certificate summaries are data-first.",
    ),
    unsupported_features=(
        "exhaustive transcription of all map callouts and lot dimensions in v1",
    ),
)


FL_ESCAMBIA_RECORDED_PLAT_MODERN_ENGINEERING_MULTISHEET_LAYOUT_V1 = CavProfile(
    profile_id="fl.escambia.recorded_plat.modern_engineering_multisheet_layout.v1",
    profile_version=1,
    family_id=RECORDED_PLAT_FAMILY_ID,
    revision="modern_engineering_multisheet_layout_v1",
    jurisdiction="FL",
    county="Escambia",
    issuing_authority="Escambia County Clerk / Recorder",
    display_name="Escambia Recorded Plat - Modern Engineering Multisheet Layout V1",
    supported_variants=(
        "multi-sheet engineering plats with legends/tables and dense plan annotations",
        "modern plat book sheets with lot/curve tables and title/legend blocks",
    ),
    coverage_notes=(
        "County-scoped layout-class profile for modern Escambia engineering-style plat sheets.",
        "Map image is preserved as an informative figure; textual metadata/certificate summaries are data-first.",
    ),
    unsupported_features=(
        "exhaustive transcription of all map callouts and lot dimensions in v1",
        "full lot/curve table transcription in v1",
    ),
)

# Transitional aliases for local exemplar scripts/notes. Canonical profile claims
# attach to the layout-class profiles above, not per-document exemplars.
FL_ESCAMBIA_RECORDED_PLAT_PB1_PG85_RENZ_ANNA_VILLA_EXEMPLAR_V1 = (
    FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1
)
FL_ESCAMBIA_RECORDED_PLAT_PB1_PG87_KUFRIANT_PARK_EXEMPLAR_V1 = (
    FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1
)
FL_ESCAMBIA_RECORDED_PLAT_PB1_PG89_SAUFLEY_HEIGHTS_EXEMPLAR_V1 = (
    FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1
)
FL_ESCAMBIA_RECORDED_PLAT_PB20_PG13B_PRESERVE_AT_DEER_RUN_PHASE_TWO_EXEMPLAR_V1 = (
    FL_ESCAMBIA_RECORDED_PLAT_MODERN_ENGINEERING_MULTISHEET_LAYOUT_V1
)


def _certificate_rows(items: Iterable[Mapping[str, Any]]) -> list[Any]:
    rows: list[Any] = []
    for item in items:
        seal_present = "Yes" if bool(item.get("seal_present")) else "No"
        signer_text = str(item.get("signer_line") or "[Illegible/Not transcribed]")
        signature_status = str(item.get("signature_status") or "present")
        signature_method = str(item.get("signature_method") or "wet_ink_scan")
        signer_ref = str(item.get("id") or item.get("block") or "certificate-block").strip().lower()
        signer_ref = signer_ref.replace(" ", "-")
        signer_semantics = el(
            "span",
            signer_text,
            data_fb_a11y_signature_status=signature_status,
            data_fb_a11y_signature_method=signature_method,
            data_fb_a11y_signature_ref=f"plat-{signer_ref}",
            class_name="plat-signature-status",
        )
        rows.append(
            SemanticTableRow(
                RowHeader(str(item.get("block") or "Certificate block")),
                DataCell(str(item.get("heading") or "[Heading not transcribed]")),
                DataCell(str(item.get("summary") or "[See source plat image]")),
                DataCell(signer_semantics),
                DataCell(seal_present),
            )
        )
    return rows


@dataclass
class RecordedPlatCavKit(CavKitBase):
    family_id: str = RECORDED_PLAT_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "title",
        "subtitle_lines",
        "plat_metadata",
        "plan_image",
        "certificate_blocks",
        "recording_annotations",
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
        return _recorded_plat_document(dict(payload))


def _figure_block(plan_image: Mapping[str, Any]) -> object:
    src = str(plan_image.get("src") or "").strip()
    alt = str(plan_image.get("alt") or "Recorded plat map image")
    caption = str(plan_image.get("caption") or "Recorded plat map image from source scan")
    if not src:
        return Box(
            Heading("Recorded plat map image", level=3),
            Text("[Source image preview unavailable]", tag="p"),
            class_name="plat-card plat-figure-fallback",
        )
    img = el(
        "img",
        src=src,
        alt=alt,
        class_name="plat-image",
        style="width:100%;height:auto;display:block;",
    )
    return Figure(
        img,
        FigCaption(caption),
        class_name="plat-card plat-figure",
    )


@Document(
    page="LETTER",
    margin="0.22in",
    title="Recorded Plat CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _recorded_plat_document(payload: dict[str, Any]) -> object:
    meta = payload.get("plat_metadata") or {}
    recording = payload.get("recording_annotations") or {}
    certs = list(payload.get("certificate_blocks") or [])
    subtitle_lines = [str(x) for x in (payload.get("subtitle_lines") or []) if str(x).strip()]

    return Stack(
        Box(
            Heading(str(payload.get("title") or "Recorded Plat"), level=1),
            *[Text(line, tag="p", class_name="plat-subtitle") for line in subtitle_lines],
            class_name="plat-card plat-title-card",
        ),
        Region(
            LayoutGrid(
                Box(
                    FieldGrid(
                        FieldItem("Jurisdiction", meta.get("jurisdiction") or "[Not transcribed]"),
                        FieldItem("County", meta.get("county") or "[Not transcribed]"),
                        FieldItem("Plat book/page", meta.get("plat_book_page") or "[Not transcribed]"),
                        FieldItem("Sheet notation", meta.get("sheet_notation") or "[Not transcribed]"),
                        FieldItem("Instrument/page mark", meta.get("margin_marking") or "[Not transcribed]"),
                        FieldItem("Prepared by", meta.get("prepared_by") or "[Not transcribed]"),
                    ),
                    class_name="plat-card plat-meta-card",
                ),
                Box(
                    FieldGrid(
                        FieldItem(
                            "Recorder annotation summary",
                            recording.get("summary") or "[Not transcribed]",
                        ),
                        FieldItem(
                            "Visible recorder/certificate blocks",
                            ", ".join(str(x) for x in (recording.get("visible_blocks") or []))
                            or "[Not transcribed]",
                        ),
                    ),
                    class_name="plat-card plat-recorder-card",
                ),
                class_name="plat-meta-grid",
            ),
            label="Recorded plat metadata and visible recorder annotations",
            class_name="plat-section",
        ),
        Section(
            Heading("Recorded Plat Map", level=2),
            _figure_block(payload.get("plan_image") or {}),
            class_name="plat-section",
        ),
        Section(
            Heading("Certificate and Dedication Blocks", level=2),
            SemanticTable(
                SemanticTableHead(
                    SemanticTableRow(
                        ColumnHeader("Block"),
                        ColumnHeader("Heading"),
                        ColumnHeader("Summary"),
                        ColumnHeader("Signer line"),
                        ColumnHeader("Seal"),
                    )
                ),
                SemanticTableBody(*_certificate_rows(certs)),
                caption="Visible certificate, dedication, and clerk/engineer/surveyor blocks transcribed from the source plat page.",
                class_name="plat-cert-table",
            ),
            class_name="plat-section",
        ),
        Text(
            "This compliant alternative version preserves the document information using a structured textual surface and an informative image of the recorded plat page.",
            tag="p",
            class_name="plat-footer-note",
        ),
        class_name="plat-root",
    )


__all__ = [
    "RECORDED_PLAT_FAMILY_ID",
    "FL_ESCAMBIA_RECORDED_PLAT_LEGACY_SIDE_CERTIFICATE_LAYOUT_V1",
    "FL_ESCAMBIA_RECORDED_PLAT_MODERN_ENGINEERING_MULTISHEET_LAYOUT_V1",
    "FL_ESCAMBIA_RECORDED_PLAT_PB1_PG85_RENZ_ANNA_VILLA_EXEMPLAR_V1",
    "FL_ESCAMBIA_RECORDED_PLAT_PB1_PG87_KUFRIANT_PARK_EXEMPLAR_V1",
    "FL_ESCAMBIA_RECORDED_PLAT_PB1_PG89_SAUFLEY_HEIGHTS_EXEMPLAR_V1",
    "FL_ESCAMBIA_RECORDED_PLAT_PB20_PG13B_PRESERVE_AT_DEER_RUN_PHASE_TWO_EXEMPLAR_V1",
    "RecordedPlatCavKit",
]
