# SPDX-License-Identifier: MIT
"""Two document families using one imported design kit, without selector wiring."""
from fullbleed.ui.experimental import Document, Flow, Footer, Grid, Header, Heading, Page, Row, style, pt, mm
from kit import Field, Label, Muted, Panel, Title, Total, WorkTable, studio_theme


def Invoice(data, *, accent="#315d9b", page=None):
    items = tuple(data["items"])
    return Document(
        Flow(
            Header(Row(Label("NORTHSTAR / STUDIO"), Label("PAID IN FULL" if data["paid"] else "INVOICE")),
                   style=style(border_bottom="1pt solid #cad3dc", padding_bottom=pt(14))),
            Flow(Title("Good work. Clear numbers.", level=1),
                 Muted("A considered record of the work we made together."), gap=pt(9)),
            Panel(Grid(Field("Prepared for", data["customer"]),
                       Field("Invoice " + data["number"], "Issued " + data["issued"]),
                       Field("Payment due", data["due"]), columns="1.4fr 1fr 1fr", gap=pt(14))),
            WorkTable(items),
            Grid(Flow(Heading("Onward, together.", level=2, style=style(font_size=pt(18))),
                      Muted("Thank you for inviting us to help shape your next chapter."), gap=pt(6)),
                 Total(items, paid=data["paid"]), gap=pt(20), style=style(break_inside="avoid")),
            Footer(Muted("NORTHSTAR STUDIO / FICTIONAL DEMONSTRATION / " + data["number"]),
                   style=style(border_top="0.5pt solid #cad3dc", padding_top=pt(10))),
            gap=pt(22),
        ),
        title="Invoice " + data["number"], lang="en-US", page=page or Page("A4", margin=mm(18)),
        theme=studio_theme(accent),
    )


def OperationsReport(data, *, accent="#245541"):
    items = tuple(data["items"])
    return Document(
        Flow(
            Label("NORTHSTAR / OPERATIONS"),
            Title("A record of progress", level=1),
            Grid(Field("Reporting period", data["period"]), Field("Prepared for", data["customer"]), gap=pt(20)),
            Muted("Each entry below is synthetic work created to exercise a flowing report."),
            WorkTable(items),
            Total(items).with_parts(label=style(letter_spacing=pt(1.2))).with_style(background="#263b46"),
            gap=pt(18),
        ), title="Operations report", lang="en-US", page=Page("LETTER", margin=mm(16)), theme=studio_theme(accent),
    )
