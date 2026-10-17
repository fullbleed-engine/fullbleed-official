#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""A component-authored, tagged community guide. Requires fullbleed==2.6.0a4."""

from __future__ import annotations

from pathlib import Path

from fullbleed.ui.experimental import (
    Box, Column, DataTable, Document, Grid, Heading, Image, Node,
    Page, Paragraph, Section, component, mm,
)

ROOT = Path(__file__).resolve().parent
TITLE = "Common Ground: A place to grow, a place to belong"
IMAGE_ALT = (
    "Illustration of a shared garden: three raised planting beds beside a broad, "
    "level path, with a bench and a greenhouse."
)
SCHEDULE = (
    {"program": "Garden together", "when": "Saturdays, 10–11:30 am",
     "expect": "Plant, water or harvest. Seated tasks are available."},
    {"program": "The shared table", "when": "Saturdays, noon–1 pm",
     "expect": "Bring your lunch or share the seasonal soup. Ingredients are listed."},
    {"program": "Make & mend", "when": "First Saturday, 2–3:30 pm",
     "expect": "Bring a small repair or try a simple craft. Materials are provided."},
)


@component
def ProgramCard(title, description):
    """One reusable card, with its heading in the document's reading order."""
    return Section(
        Heading(title, level=3),
        Paragraph(description),
        class_name="program-card",
    )


@component
def BulletList(*items):
    # Node is the public escape hatch for semantic elements without a shortcut.
    return Node("ul", tuple(Node("li", (item,)) for item in items))


@component
def TextLink(label, href):
    return Node("a", (label,), (("href", href),))


@component
def ProgramSchedule(rows):
    return DataTable(
        rows,
        columns=(
            Column("Program", lambda row: row["program"], width=mm(38)),
            Column("When", lambda row: row["when"], width=mm(43)),
            Column("What to expect", lambda row: row["expect"]),
        ),
        caption="Autumn 2026 · Weekly sessions at the Common Ground garden",
        class_name="schedule",
    )


def build_document(*, browser=False):
    styles = (ROOT / "guide.css").read_text(encoding="utf-8")
    if browser:
        styles += "\n" + (ROOT / "browser.css").read_text(encoding="utf-8")
    return Document(
        Section(
            Paragraph("Common Ground", class_name="brand"),
            Paragraph("Community field guide / Autumn 2026", class_name="eyebrow"),
            Heading("A place to grow.", Node("br"), "A place to belong.", level=1,
                    class_name="cover-title"),
            Paragraph(
                "Good things happen when we make a little room for one another. "
                "Come for a quiet hour, a shared meal or something new to learn.",
                class_name="intro",
            ),
            Image(src="garden.svg", alt=IMAGE_ALT, class_name="garden"),
            Heading("Find your starting point", level=2),
            Grid(
                ProgramCard("Grow something",
                            "A few seeds. A pair of hands. A garden we tend together."),
                ProgramCard("Share a table",
                            "Pull up a chair for conversation and a warm bowl of soup."),
                ProgramCard("Make it last",
                            "Swap a skill, mend a favorite thing or try a small project."),
                columns=3, class_name="programs",
            ),
            Box(
                Heading("Every visit starts with hello", level=2),
                Paragraph("All sessions are free. No membership, equipment or experience is needed. "
                          "Come on your own or bring someone with you."),
                class_name="welcome",
            ),
            class_name="sheet cover",
        ),
        Section(
            Paragraph("Your next Saturday", class_name="eyebrow"),
            Heading("Make room for a good day.", level=2, class_name="page-title"),
            Paragraph("Stay for one session or linger a little longer. You decide how to take part.",
                      class_name="intro"),
            ProgramSchedule(SCHEDULE),
            Heading("Plan a comfortable visit", level=2),
            Grid(
                Section(
                    Heading("An easy arrival", level=3),
                    BulletList(
                        "Enter through the wide gate beside the greenhouse.",
                        "The main path is firm and level. Seating is available along the way.",
                        "A welcome volunteer can show you the garden and the accessible restroom.",
                    ),
                ),
                Section(
                    Heading("Choose your pace", level=3),
                    BulletList(
                        "Take part seated or standing, or simply watch.",
                        "Use the quiet corner whenever you would like a break.",
                        "Ask for written instructions or a demonstration. Both are welcome.",
                    ),
                ),
                columns=2, class_name="visit",
            ),
            Box(
                Heading("A small first step is enough", level=2),
                Paragraph("You do not have to know anyone, know what to say or stay all day. "
                          "There is a place for you here."),
                class_name="closing",
            ),
            Section(
                Heading("About this guide", level=2),
                Paragraph("Common Ground and its programs are fictional. This sample shows how "
                          "a thoughtfully designed document can carry its structure into PDF and HTML."),
                Paragraph(TextLink("Fullbleed documentation: docs.fullbleed.dev",
                                   "https://docs.fullbleed.dev/")),
                class_name="about",
            ),
            class_name="sheet details",
        ),
        title=TITLE,
        lang="en-US",
        page=Page(size="A4", margin=mm(0)),
        styles=styles,
    )
