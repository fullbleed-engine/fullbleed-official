# SPDX-License-Identifier: MIT
"""A fictional seasonal brief built from reusable component values."""
from fullbleed.ui.experimental import (
    Box, Column, DataTable, Document, Flow, Heading, Page, PageBand, PageBreak,
    PageNumbers, Paragraph, Row, Stack, Strong, Theme, component, mm, pt, style,
    styled, token,
)


THEME = Theme(
    {"ink": "#183e34", "muted": "#53665c", "paper": "#edf2e6", "accent": "#755122"},
    text=style(font_family="Inter", font_size=pt(10), line_height=1.45, color=token("ink")),
)
Kicker = styled(Paragraph, font_size=pt(7), font_weight=700, letter_spacing=pt(1),
                text_transform="uppercase", color=token("accent"))
Title = styled(Heading, font_size=pt(31), line_height=1.08, letter_spacing=pt(-0.8))
Small = styled(Paragraph, font_size=pt(8), color=token("muted"))


@component(parts=("name", "detail"))
def Brand():
    return Stack(
        Paragraph("COMMON GROUND", part="name", style=style(font_size=pt(8), font_weight=700, letter_spacing=pt(0.8))),
        Paragraph("A neighborhood, growing together.", part="detail", style=style(font_size=pt(6), color=token("muted"))),
        gap=pt(2),
    )


@component(parts=("value", "label"))
def Metric(value, label):
    return Stack(
        Paragraph(value, part="value", style=style(font_size=pt(32), font_weight=700, line_height=1.1)),
        Small(label, part="label"), gap=pt(5),
    )


@component(parts=("heading", "body"))
def NextStep(number, title, text):
    return Stack(
        Kicker(number),
        Heading(title, level=3, part="heading", style=style(font_size=pt(16))),
        Paragraph(text, part="body"), gap=pt(6),
    )


def build_document():
    activity = (
        ("Garden mornings", "12", "84 visits"),
        ("Repair afternoons", "4", "31 items repaired"),
        ("Shared meals", "3", "96 places set"),
        ("Seed exchanges", "2", "47 packets shared"),
        ("Welcome walks", "6", "28 neighbors"),
    )
    return Document(
        Flow(
            Kicker("Field notes / Autumn 2026"),
            Title("A season of shared space.", level=1),
            Paragraph("A small record of what happens when neighbors have a place to meet, make, and lend a hand."),
            Box(
                Row(Metric("126", "neighbors took part"), Metric("21", "open sessions"), gap=pt(18)),
                style=style(background=token("paper"), padding=pt(18)),
            ),
            Heading("The place is the invitation", level=2, style=style(font_size=pt(17))),
            Paragraph("We kept the gate open, the table long, and the activities simple. New faces became familiar ones. Practical work made room for conversation."),
            Small("Common Ground is a fictional community project. Every figure in this brief is sample data."),
            gap=pt(15),
        ),
        PageBreak(),
        Flow(
            Kicker("01 / What we made possible"),
            Title("Small actions. Shared results.", level=2),
            Paragraph("The busiest sessions mixed a useful task with time to stay. No specialist knowledge was needed to join in."),
            DataTable(
                activity, caption="Season activity", columns=(
                    Column("Activity", lambda row: row[0]),
                    Column("Sessions", lambda row: row[1], align="right", width=mm(18)),
                    Column("Participation", lambda row: row[2], width=mm(37)),
                ),
            ).with_parts(
                caption=style(font_size=pt(9), font_weight=700, margin_bottom=pt(7)),
                header=style(background=token("paper"), font_size=pt(8), color=token("ink")),
                cell=style(font_size=pt(8), padding=pt(8)),
            ),
            Box(
                Kicker("A useful lesson"),
                Paragraph(Strong("Make the first step visible. "), "A welcome at the gate, a clearly marked tool bench, and a task that takes ten minutes helped people begin."),
                style=style(background=token("paper"), padding=pt(15)),
            ),
            Small("Visits and participation counts can include the same neighbor more than once. They describe activity, not unique attendance."),
            gap=pt(12),
        ),
        PageBreak(),
        Flow(
            Kicker("02 / What comes next"),
            Title("Keep the welcome growing.", level=2),
            Paragraph("Three modest commitments will guide the next season. Each has a clear owner and a way for someone new to help."),
            NextStep("01", "Make room at the table", "Reserve a few places at every shared meal for neighbors joining for the first time. Publish the menu and access information together."),
            NextStep("02", "Pass on a practical skill", "Pair each repair session with a short demonstration. Keep a simple printed guide beside the tools for people who prefer to follow at their own pace."),
            NextStep("03", "Listen before adding more", "Ask what made participation easy and what got in the way. Use those answers to choose the next opening hours and activities."),
            Small("Prepared by Common Ground / October 2026. This sample is authored with Fullbleed components, including its repeated header and finalized page numbers."),
            gap=pt(15),
        ),
        title="Common Ground — Field notes", lang="en-US", theme=THEME,
        page=Page(
            "A5", margin=(mm(20), mm(17), mm(20), mm(17)),
            header=PageBand(left=Brand(), right=Kicker("Field notes / 03").with_style(font_size=pt(6))),
            footer=PageBand(
                left=Small("AUTUMN 2026 · SAMPLE DATA").with_style(font_size=pt(6)),
                right=PageNumbers(total=True, prefix="", separator=" / ", class_name="folio"),
            ),
        ),
        styles={".folio": style(font_size=pt(8), color=token("muted"))},
    )
