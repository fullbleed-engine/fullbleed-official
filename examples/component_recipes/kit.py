# SPDX-License-Identifier: MIT
"""A small design kit shared by the invoice and the operations report."""
from fullbleed.ui.experimental import (
    Card, Column, DataTable, Flow, Heading, Paragraph, Stack, Strong,
    Theme, component, pt, mm, style, styled, token,
)


STUDIO_THEME = Theme({"accent": "#315d9b", "ink": "#263b46", "muted": "#667580", "paper": "#f2f5f8"},
                     text=style(font_family="Inter", font_size=pt(9), line_height=1.45, color=token("ink")))


def studio_theme(accent="#315d9b"):
    return STUDIO_THEME.with_values(accent=accent)


Label = styled(Paragraph, font_size=pt(7), font_weight=700, letter_spacing=pt(0.8), text_transform="uppercase")
Muted = styled(Paragraph, font_size=pt(8), color=token("muted"))
Title = styled(Heading, font_size=pt(36), line_height=1.05, letter_spacing=pt(-1), color=token("accent"))
Panel = styled(Card, border="none", border_radius=pt(0), padding=pt(16), background=token("paper"))


@component(parts=("label", "value"))
def Field(label, *value):
    return Stack(Label(label, part="label"), Paragraph(*value, part="value"), gap=pt(4))


def money(cents):
    sign = "-" if cents < 0 else ""
    return f"{sign}${abs(cents) // 100:,}.{abs(cents) % 100:02d}"


@component
def WorkCell(item):
    return Flow(Paragraph(Strong(item["name"])), Muted(item["detail"]), gap=pt(3))


@component
def WorkTable(items):
    return DataTable(items, caption="The work, in detail", columns=[
        Column("Project", WorkCell),
        Column("Hours", lambda item: item["quantity"], align="right", width=mm(17)),
        Column("Rate", lambda item: money(item["rate_cents"]), align="right", width=mm(27)),
        Column("Amount", lambda item: money(item["quantity"] * item["rate_cents"]), align="right", width=mm(30)),
    ], empty="No billable items.").with_parts(
        header=style(font_size=pt(7), background=token("paper"), color=token("ink")),
        caption=style(font_size=pt(8), font_weight=700),
    )


@component(parts=("label", "amount", "note"))
def Total(items, *, paid=False):
    cents = sum(item["quantity"] * item["rate_cents"] for item in items)
    return Panel(
        Label("TOTAL PAID · USD" if paid else "TOTAL DUE · USD", part="label"),
        Paragraph(money(cents), style=style(font_size=pt(28), font_weight=700, margin_top=pt(6)), part="amount"),
        Paragraph("Thank you." if paid else "Payment details supplied separately.", style=style(font_size=pt(7)), part="note"),
        style=style(background=token("accent"), color="#ffffff"),
    )
