# SPDX-License-Identifier: MIT
"""Dependency-free authoring vocabulary shared by runtime checks and type stubs.

This is the experimental API's checked vocabulary, not an engine CSS capability
declaration. Additional CSS remains available through the explicit css() helper.
"""

STYLE_PROPERTIES = frozenset("""
    color background background-color font-family font-size font-weight font-style
    font-variant-numeric line-height letter-spacing text-align text-transform
    text-decoration white-space vertical-align display flex-direction flex-grow
    flex-shrink flex-basis flex-wrap align-items align-self justify-content gap
    row-gap column-gap grid-template-columns grid-column grid-row width height
    min-width max-width min-height max-height margin margin-top margin-right
    margin-bottom margin-left padding padding-top padding-right padding-bottom
    padding-left border border-top border-right border-bottom border-left
    border-width border-style border-color border-radius border-collapse
    box-sizing table-layout break-before break-after break-inside orphans widows
    overflow opacity object-fit list-style-type
""".split())

LENGTH_PROPERTIES = frozenset("""
    font-size letter-spacing gap row-gap column-gap width height min-width
    max-width min-height max-height margin margin-top margin-right margin-bottom
    margin-left padding padding-top padding-right padding-bottom padding-left
    border-width border-radius
""".split())

PRIMITIVE_TAGS = {
    "Box": "div", "Paragraph": "p", "Text": "span", "Strong": "strong",
    "Emphasis": "em", "Section": "section", "Header": "header",
    "Footer": "footer", "Card": "section",
}
ALIGNMENTS = ("stretch", "start", "end", "center", "baseline")
JUSTIFICATIONS = ("start", "end", "center", "space-between", "space-around", "space-evenly")
COLUMN_ALIGNMENTS = ("left", "right", "center")
ROW_BREAKS = ("avoid", "auto")
TABLE_PARTS = ("header", "cell", "row", "caption", "empty")
