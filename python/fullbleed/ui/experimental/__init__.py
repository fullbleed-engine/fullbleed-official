# SPDX-License-Identifier: MIT
"""Experimental document components. APIs may change before stabilization.

Ordinary functions, immutable style recipes, explicit theme tokens, named parts,
page geometry and data tables compile to Fullbleed HTML/CSS. Importing this module
does not load fonts, start a renderer, or register global styles.
"""
from ._style import AuthoringError, Length, Style, Theme, Token, css, mm, pct, pt, style, token
from ._tree import (
    Box, Card, Emphasis, Flow, Footer, Fragment, Grid, Header, Heading, Image,
    ComponentFactory, Node, PageBreak, Paragraph, Row, Stack, Strong, Text, Section, component, styled,
)
from ._document import DocumentSource, Document, LayoutError, Page, Renderer, Source
from ._table import Column, DataTable
from ._bridge import to_legacy

__all__ = [
    "AuthoringError", "Length", "Style", "Theme", "Token", "css", "mm", "pct", "pt", "style", "token",
    "Box", "Card", "Emphasis", "Flow", "Footer", "Fragment", "Grid", "Header", "Heading", "Image",
    "ComponentFactory", "Node", "PageBreak", "Paragraph", "Row", "Stack", "Strong", "Text", "Section", "component", "styled",
    "DocumentSource", "Document", "LayoutError", "Page", "Renderer", "Source", "Column", "DataTable", "to_legacy",
]
