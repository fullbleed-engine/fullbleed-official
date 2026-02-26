# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from ..core import Document
from ..accessibility import (
    ColumnHeader,
    DataCell,
    Heading,
    Region,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
)
from ..primitives import Box, Stack, Text
from ._core import CavKitBase, CavProfile


INVESTMENT_PORTFOLIO_REPORT_FAMILY_ID = "investment_portfolio_report_cav"


FL_ESCAMBIA_INVESTMENT_PORTFOLIO_SUMMARY_FY2019_2020_NOV2019_V1 = CavProfile(
    profile_id="fl.escambia.investment_portfolio_summary.fy2019_2020.nov2019.v1",
    profile_version=1,
    family_id=INVESTMENT_PORTFOLIO_REPORT_FAMILY_ID,
    revision="fy2019_2020_nov2019_v1",
    jurisdiction="US",
    county="Escambia",
    issuing_authority="Escambia County Board of County Commissioners / Escambia Clerk",
    display_name="Investment Portfolio Summary Report FY 2019-2020 (November 30, 2019) V1",
    supported_variants=(
        "multi-page portfolio summary/policy compliance reports with tabular allocations, issuer tables, and cover page metadata",
    ),
    coverage_notes=(
        "Profile is layout-scoped to the 5-page Escambia investment portfolio report source from DocCenter 744.",
        "Chart visuals are represented as explicit textual percentages and table data in v1.",
    ),
    unsupported_features=(
        "native chart image recreation beyond text/percent equivalents in v1",
    ),
)


def _cells_for_row(row: Any, columns: list[str]) -> list[Any]:
    if isinstance(row, Mapping):
        return [DataCell(str(row.get(col) or "")) for col in columns]
    if isinstance(row, (list, tuple)):
        padded = list(row) + ([""] * max(0, len(columns) - len(row)))
        return [DataCell(str(v)) for v in padded[: len(columns)]]
    return [DataCell(str(row))] + [DataCell("") for _ in range(max(0, len(columns) - 1))]


def _table_node(table: Mapping[str, Any]) -> Any:
    columns = [str(c) for c in (table.get("columns") or [])]
    rows = list(table.get("rows") or [])
    head = SemanticTableHead(SemanticTableRow(*[ColumnHeader(c) for c in columns]))
    body_rows = [SemanticTableRow(*_cells_for_row(r, columns)) for r in rows]
    body = SemanticTableBody(*body_rows)
    return SemanticTable(
        head,
        body,
        caption=table.get("caption"),
        class_name=str(table.get("class_name") or "ipr-table"),
    )


def _section_node(section: Mapping[str, Any]) -> Any:
    nodes: list[Any] = []
    heading = str(section.get("heading") or "").strip()
    if heading:
        nodes.append(Heading(heading, level=2))
    for p in section.get("paragraphs") or []:
        nodes.append(Text(str(p), tag="p", class_name="ipr-body"))
    for t in section.get("tables") or []:
        if isinstance(t, Mapping):
            nodes.append(_table_node(t))
    return Section(*nodes, class_name="ipr-section")


@dataclass
class InvestmentPortfolioReportCavKit(CavKitBase):
    family_id: str = INVESTMENT_PORTFOLIO_REPORT_FAMILY_ID
    allowed_payload_fields: tuple[str, ...] = (
        "schema",
        "document_kind",
        "cover_page",
        "pages",
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
        return _investment_portfolio_report_document(dict(payload))


@Document(
    page="LETTER",
    margin="0.34in",
    title="Investment Portfolio Report CAV (Accessibility-First)",
    bootstrap=False,
    lang="en-US",
)
def _investment_portfolio_report_document(payload: dict[str, Any]) -> Any:
    cover = dict(payload.get("cover_page") or {})
    cover_nodes: list[Any] = []
    for i, line in enumerate(cover.get("title_lines") or []):
        cover_nodes.append(Heading(str(line), level=1 if i == 0 else 2))
    for p in cover.get("subtitle_lines") or []:
        cover_nodes.append(Text(str(p), tag="p", class_name="ipr-body"))
    for p in cover.get("prepared_by_lines") or []:
        cover_nodes.append(Text(str(p), tag="p", class_name="ipr-body"))
    if str(cover.get("footer_note") or "").strip():
        cover_nodes.append(Text(str(cover.get("footer_note")), tag="p", class_name="ipr-footer-note"))

    pages_out: list[Any] = [
        Region(
            Box(*cover_nodes, class_name="ipr-cover-box"),
            label=str(cover.get("page_label") or "Investment portfolio report cover"),
            class_name="ipr-page ipr-page-1",
        )
    ]

    for idx, page in enumerate(payload.get("pages") or [], start=2):
        page_map = dict(page or {})
        page_nodes: list[Any] = []
        title = str(page_map.get("title") or "").strip()
        if title:
            page_nodes.append(Heading(title, level=1))
        for p in page_map.get("intro_paragraphs") or []:
            page_nodes.append(Text(str(p), tag="p", class_name="ipr-body"))
        for section in page_map.get("sections") or []:
            if isinstance(section, Mapping):
                page_nodes.append(_section_node(section))
        pages_out.append(
            Region(
                *page_nodes,
                label=str(page_map.get("page_label") or f"Investment portfolio report page {idx}"),
                class_name=f"ipr-page ipr-page-{idx} page-break-before",
            )
        )

    return Stack(*pages_out, class_name="ipr-root")


__all__ = [
    "INVESTMENT_PORTFOLIO_REPORT_FAMILY_ID",
    "FL_ESCAMBIA_INVESTMENT_PORTFOLIO_SUMMARY_FY2019_2020_NOV2019_V1",
    "InvestmentPortfolioReportCavKit",
]
