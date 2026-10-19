# SPDX-License-Identifier: MIT
"""Immutable page-furniture authoring values; pagination stays in the engine."""
from __future__ import annotations

from dataclasses import dataclass

from ._style import AuthoringError, Style
from ._tree import Node, _children

_SLOTS = ("left", "center", "right")


def _css_string(value: str) -> str:
    # CSS strings use hexadecimal escapes for controls, not JSON's backslash-n.
    return '"' + ''.join(
        '\\' + character if character in {'"', '\\'}
        else f'\\{ord(character):x} ' if ord(character) < 32 or ord(character) == 127
        else character for character in value
    ) + '"'


@dataclass(frozen=True, init=False)
class PageNumbers:
    """A PageBand slot value supplied by finalized native pagination.

    Browser HTML contains explicit [page]/[pages] placeholders. It does not run
    pagination or fabricate page counts. Root styles and authored classes use
    native text/container styling. Author labels with prefix/separator/suffix.
    """

    total: bool
    prefix: str
    separator: str
    suffix: str
    style: Style
    class_name: str | None

    def __init__(self, *, total=False, prefix="Page ", separator=" of ", suffix="", style=None, class_name=None):
        if not isinstance(total, bool):
            raise AuthoringError("total must be a boolean", code="page_numbers.total")
        for name, value in (("prefix", prefix), ("separator", separator), ("suffix", suffix)):
            if not isinstance(value, str) or '\x00' in value:
                raise AuthoringError(f"{name} must be a literal string without NUL", code="page_numbers.label")
        if class_name is not None and not isinstance(class_name, str):
            raise AuthoringError("class_name must be a string", code="page_numbers.class")
        captured = Style(style)
        if any(name == 'position' for name, _ in captured.entries):
            raise AuthoringError("PageBand owns the position of PageNumbers; style its text and box instead.", code="page_numbers.style")
        for name, value in dict(total=total, prefix=prefix, separator=separator, suffix=suffix,
                                style=captured, class_name=class_name).items():
            object.__setattr__(self, name, value)

    def _content_css(self) -> str:
        parts = [_css_string(self.prefix), "counter(page)"]
        if self.total:
            parts.extend((_css_string(self.separator), "counter(pages)"))
        parts.append(_css_string(self.suffix))
        return ' '.join(parts)

    def _html_node(self, target) -> Node:
        children = [self.prefix, Node("span", ("[page]" if target == 'html' else '0',), attributes=(("data-fb-page-counter", "page"),))]
        if self.total:
            children.extend((self.separator, Node("span", ("[pages]" if target == 'html' else '0',), attributes=(("data-fb-page-counter", "pages"),))))
        children.append(self.suffix)
        attributes = (("class", self.class_name),) if self.class_name else ()
        return Node("div", tuple(children), attributes=attributes, style=self.style, name="PageNumbers")


@dataclass(frozen=True, init=False)
class PageBand:
    """Reusable left/center/right content in a Page's reserved header or footer.

    Ordinary components keep their styles, resources and authored classes.
    Content must fit the existing Page margins; it is never scaled to fit.
    first_page and blank_pages affect PDF repetition, not the single HTML band.
    """

    left: tuple | PageNumbers
    center: tuple | PageNumbers
    right: tuple | PageNumbers
    first_page: bool
    blank_pages: bool

    def __init__(self, *, left=None, center=None, right=None, first_page=True, blank_pages=False):
        for name, value in (("first_page", first_page), ("blank_pages", blank_pages)):
            if not isinstance(value, bool):
                raise AuthoringError(f"{name} must be a boolean", code="page_band.policy")
            object.__setattr__(self, name, value)
        for name, value in (("left", left), ("center", center), ("right", right)):
            try:
                captured = value if isinstance(value, PageNumbers) else _children((value,))
            except AuthoringError as error:
                raise error.at(f"PageBand.{name}") from error
            object.__setattr__(self, name, captured)

    def _items(self):
        return tuple((name, value) for name in _SLOTS if (value := getattr(self, name)))

    def _tokens(self):
        from ._tree import _required_tokens
        for _, value in self._items():
            yield from value.style._token_names() if isinstance(value, PageNumbers) else _required_tokens(value)


def band_nodes(page, theme, target):
    """Build roots for the Document's existing traversal, never a second serializer."""
    groups, slots = {}, {}
    for edge, band in (("header", page.header), ("footer", page.footer)):
        if band is None:
            continue
        children = []
        for position, value in band._items():
            number = value if isinstance(value, PageNumbers) else None
            if number is None:
                wrapper = Node("div", value, name=f"Page.{edge}.{position}",
                               _defaults=Style(theme.text, text_align=position),
                               attributes=(("data-fb-band-slot", f"{edge}-{position}"),))
            else:
                from dataclasses import replace
                base = number._html_node(target)
                wrapper = replace(base, name=f"Page.{edge}.{position}",
                                  _defaults=Style(theme.text, text_align=position),
                                  attributes=(*base.attributes, ("data-fb-band-slot", f"{edge}-{position}")))
            slots[id(wrapper)] = (edge, position, band, number)
            children.append(wrapper)
        if children:
            groups[edge] = Node(None, tuple(children)) if target == "pdf" else Node(
                edge, tuple(children), attributes=(("data-fb-page-band", edge),), name=f"Page.{edge}")
    return groups, slots


def slot_css(edge, position, band, number, source_id):
    slot = ("top" if edge == "header" else "bottom") + "-" + position
    if number is None:
        content = f"element(fbx-{edge}-{position})"
        declarations = ""
    else:
        content = number._content_css()
        declarations = f"-fb-page-band: {_css_string(source_id)};"
    rules = [f"@page {{ @{slot} {{ content: {content}; {declarations} }} }}"]
    if not band.first_page:
        rules.append(f"@page:first {{ @{slot} {{ content: none; }} }}")
    if not band.blank_pages:
        rules.append(f"@page:blank {{ @{slot} {{ content: none; }} }}")
    return rules


HTML_BAND_CSS = '''[data-fb-page-band] { display:flex; justify-content:space-between; align-items:center; gap:1em; }
[data-fb-page-band] > [data-fb-band-slot] { flex:1; min-width:0; }
[data-fb-page-counter] { font-style:italic; }
'''
