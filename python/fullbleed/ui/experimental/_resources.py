# SPDX-License-Identifier: MIT
"""Immutable, document-local dependencies carried by component trees."""
from __future__ import annotations

from dataclasses import dataclass
import re

from ._style import AuthoringError


@dataclass(frozen=True)
class Resource:
    name: str
    kind: str
    text: str

    def __post_init__(self):
        if not isinstance(self.name, str) or not self.name.strip():
            raise AuthoringError("Component resources require a name", code="resource.name")
        if self.kind != "css" or not isinstance(self.text, str):
            raise AuthoringError("Component resources require CSS text", code="resource.type")


def raw_text(text: str, tag: str) -> str:
    """Keep HTML raw-text end tags inside their CSS value or comment.

    Escaping the slash preserves quoted CSS strings. HTML entities would not
    decode in these elements and would change the stylesheet.
    """
    return re.sub(r"</(?=" + tag + r"[\t\n\f\r />])", r"<\\/", text, flags=re.IGNORECASE)


def style_element(css: str) -> str:
    return "<style>" + raw_text(css, "style") + "</style>" if css else ""
