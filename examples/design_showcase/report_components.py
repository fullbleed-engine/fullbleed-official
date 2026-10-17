# SPDX-License-Identifier: MIT
"""Common Ground: a three-page editorial report composed in Python."""
from __future__ import annotations

from decimal import Decimal

from fullbleed.ui.experimental import (
    Box, Column, DataTable, Document, Fragment, Header, Heading, Node,
    Paragraph, Section, Strong, Text, component, pct, pt, style,
)
from component_primitives import Folio, Lines, VectorArtwork, design_document, money


@component
def ReportMasthead(section: str | Node, *, cover: bool = False) -> Node:
    brand = Lines("Common Ground", Text("COMMUNITY TRUST", class_name="kicker")) if cover else "Common Ground"
    return Header(Box(brand, class_name="brand"), Box(section, class_name="kicker"), class_name="masthead")


@component
def ReportPage(*content, section: str | Node, footer: str, number: str, cover: bool = False) -> Node:
    return Section(ReportMasthead(section, cover=cover), *content, Folio(footer, number), class_name="page report")


@component
def ContentsEntry(label: str, *lines) -> Node:
    return Box(Box(label, class_name="kicker"), Paragraph(Lines(*lines), class_name="serif"))


@component
def CoverPage() -> Node:
    return ReportPage(
        Heading(Lines("COMMON", "GROUND"), level=1, class_name="cover-title"),
        Box(Paragraph(Lines("Stronger places.", "More room", "for everyone.")),
            Box(Box("2026", class_name="year"),
                Paragraph("COMMUNITY REVIEW", class_name="kicker",
                          style=style(font_family="Inter", font_size=pt(8), text_align="right", margin_top=pt(8)))),
            class_name="cover-deck"),
        VectorArtwork("landscape.svg", class_name="landscape"),
        Box(Box(Lines("A YEAR", "IN COMMON"), class_name="kicker", style=style(color="#b44123")),
            ContentsEntry("02 / OUR REACH", "Small actions,", "shared outcomes."),
            ContentsEntry("03 / OUR RESOURCES", "Where the", "support goes."), class_name="contents"),
        section=Lines("PEOPLE", "PLACES", "POSSIBILITIES"), cover=True,
        footer="COMMON GROUND / COMMUNITY REVIEW", number="FICTIONAL DATA · 01 / 03",
    )


@component
def SectionIntro(kicker: str, title: tuple[str, ...], description: str) -> Node:
    return Fragment(
        Paragraph(kicker, class_name="kicker", style=style(color="#b44123")),
        Heading(Lines(*title), level=1, class_name="section-title"),
        Paragraph(description, class_name="deck"),
    )


@component(parts=("value", "caption"))
def Metric(value: int, *caption: str) -> Node:
    return Box(Box(f"{value:,}", class_name="value", part="value"),
               Paragraph(Lines(*caption), class_name="metric-caption", part="caption"))


@component
def ParticipationChart(values, *, maximum: int = 980) -> Node:
    values = tuple(values)
    # Use the same explicit scale as the original HTML authoring path.
    if maximum <= 0:
        raise ValueError("The chart maximum must be positive")
    return Section(
        Box(Heading("More neighbors, each quarter."), Text("PARTICIPATING HOUSEHOLDS", class_name="kicker"),
            class_name="row"),
        Box((Box(Box(f"{value:,}", class_name="chart-value"),
                 Box(class_name="bar", style=style(height=pt(round(value / maximum * 105, 2)))),
                 class_name="chart-col") for value in values), class_name="chart-bars"),
        Box((Text(f"Q{index}") for index in range(1, len(values) + 1)), class_name="chart-labels"),
        Paragraph("Quarterly participation counts; a household can participate in more than one quarter.",
                  class_name="small", style=style(margin_top=pt(12), color="#c4d4bd")),
        class_name="chart-panel",
    )


@component
def ReachPage(report) -> Node:
    return ReportPage(
        SectionIntro("A PLACE TO BEGIN", ("Small actions.", "Shared outcomes."),
                     "An open door, a working kitchen, a garden plot. This year, our fictional neighborhood "
                     "programs connected more people with practical spaces to learn, make, and belong."),
        Box(Metric(report["households"], "households took part", "across four neighborhoods"),
            Metric(report["volunteer_hours"], "volunteer hours", "shared across our programs"),
            Metric(report["spaces"], "community spaces", "kept open and active"), class_name="metrics grid-3"),
        ParticipationChart(report["quarterly_households"]),
        Box(Box(Paragraph(Lines("“Belonging starts", "with somewhere", "to show up.”"), class_name="quote"),
                Paragraph("OUR PROGRAM PRINCIPLE", class_name="kicker", style=style(margin_top=pt(12)))),
            Box(Heading("Built around daily life."),
                Paragraph("Evening workshops, shared meals, and weekend garden sessions made participation easier "
                          "for people with different schedules. The next step is a longer opening window at each "
                          "neighborhood space.", class_name="note")), class_name="grid-2"),
        section="02 / OUR REACH", footer="ILLUSTRATIVE ANNUAL REVIEW / NOT AN ACTUAL ORGANIZATION", number="02 / 03",
    )


@component
def AllocationShare(amount, total) -> Node:
    share = Decimal(amount) / total * 100 if total else Decimal(0)
    return Fragment(f"{share:.0f}%",
                    Box(Box(class_name="share-fill", style=style(width=pct(share))),
                        class_name="share-track", style=style(margin_left="auto")))


@component
def AllocationTable(allocations, total) -> Node:
    return DataTable(
        allocations,
        columns=(
            Column("PROGRAM AREA", lambda item: Strong(item["name"]), width=pct(50)),
            Column("ALLOCATION", lambda item: money(item["amount"]), align="right"),
            Column("SHARE", lambda item: AllocationShare(item["amount"], total), align="right", width=pt(88)),
        ),
        class_name="funding-table", style=style(table_layout="auto", margin_top=pt(25)),
    ).with_parts(
        header=style(padding=(pt(10), 0), border_bottom=".55pt solid #bdc8bf", font_weight=600),
        cell=style(padding=(pt(8), 0), border_bottom=".55pt solid #bdc8bf"),
    )


@component
def ResourcesPage(report) -> Node:
    allocations = tuple(report["allocations"])
    total = sum((Decimal(item["amount"]) for item in allocations), Decimal(0))
    return ReportPage(
        SectionIntro("FOLLOW THE SUPPORT", ("Good intentions.", "Clear allocations."),
                     "A practical view of this fictional operating budget. Every category supports the same aim: "
                     "useful places, open to more people."),
        AllocationTable(allocations, total),
        Box(Text("TOTAL OPERATING ALLOCATION · USD", class_name="kicker"), Strong(money(total)),
            class_name="allocation-total"),
        Box(Section(Heading("What we count."),
                    Paragraph("Households are deduplicated within the annual program register. Volunteer hours "
                              "include recorded event and maintenance time. Space counts refer to year-round "
                              "locations, not individual events.", class_name="note"),
                    Paragraph("Quarterly counts show participation within each period and must not be added to "
                              "estimate unique annual reach.", class_name="note", style=style(margin_top=pt(11)))),
            Section(Heading("What comes next."),
                    Paragraph("Extend evening access. Improve step-free entrances. Publish a simple monthly view "
                              "of program activity and spending.", class_name="note"),
                    Paragraph(Lines("A shared tomorrow", "is built in the everyday."), class_name="serif",
                              style=style(font_size=pt(21), line_height=1.15, margin_top=pt(19), color="#b44123")),
                    class_name="callout"), class_name="grid-2"),
        section="03 / OUR RESOURCES", footer="COMMON GROUND / METHODS & ALLOCATIONS", number="FICTIONAL DATA · 03 / 03",
    )


def common_ground_report(report) -> Document:
    return design_document(CoverPage(), ReachPage(report), ResourcesPage(report),
                           kind="report", title="Common Ground — Community Review 2026")
