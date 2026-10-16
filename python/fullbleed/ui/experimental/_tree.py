# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Iterable, Mapping, Set
from dataclasses import dataclass, replace
from decimal import Decimal
from functools import wraps
from typing import Callable, ParamSpec, Protocol, cast, overload

from ..core import _normalize_attr_name, _ATTR_NAME, _TAG_NAME, _HTML_VOID_ELEMENTS
from ._style import AuthoringError, Style, _number, pt, style
from ._schema import ALIGNMENTS, JUSTIFICATIONS, PRIMITIVE_TAGS

P = ParamSpec("P")


def _declared_parts(values) -> tuple[str, ...] | None:
    if values is None:
        return None
    if isinstance(values, (str, bytes, Mapping, Set)) or not isinstance(values, Iterable):
        raise AuthoringError("Declare parts as an ordered iterable of names", code="component.parts")
    names = tuple(values)
    if any(not isinstance(name, str) or not name.isidentifier() for name in names):
        raise AuthoringError("Declared part names must be identifiers, such as 'label'", code="component.parts")
    if len(set(names)) != len(names):
        raise AuthoringError("Declared part names must be unique", code="component.parts")
    return names


def _owned_parts(children):
    """Authored markers in this component, excluding other component boundaries."""
    pending = list(reversed(children))
    while pending:
        node = pending.pop()
        if not isinstance(node, Node) or node.boundary:
            continue
        if node.part is not None:
            yield node.part
        pending.extend(reversed(node.children))


def _children(values) -> tuple:
    output, active = [], set()

    def visit(value):
        if value is None or isinstance(value, bool):
            return
        if isinstance(value, (str, Node)):
            output.append(value)
        elif isinstance(value, (int, float, Decimal)):
            output.append(_number(value))
        elif isinstance(value, Iterable) and not isinstance(value, (Mapping, Set, bytes, bytearray)):
            identity = id(value)
            if identity in active:
                raise AuthoringError("Cyclic children", code="children.cycle")
            active.add(identity)
            try:
                for child in value:
                    visit(child)
            finally:
                active.remove(identity)
        else:
            raise AuthoringError(f"Unsupported child {type(value).__name__}; format data or call its component", code="children.type")

    visit(values)
    return tuple(output)


def _attrs(values):
    output = {}
    for name, value in (values or {}).items():
        normalized = _normalize_attr_name(name).lower()
        if not _ATTR_NAME.fullmatch(normalized) or normalized in {"style", "data-fb-id"}:
            raise AuthoringError(f"Reserved or invalid attribute {name!r}", code="attribute.name")
        if normalized in output:
            raise AuthoringError(f"Duplicate attribute {normalized!r}", code="attribute.duplicate")
        if value is None:
            continue
        if normalized == "class" and not isinstance(value, str):
            raise AuthoringError("Class names must be strings", code="attribute.value")
        if not isinstance(value, (str, bool, int, float, Decimal)):
            raise AuthoringError(f"Attribute {name!r} must be a scalar", code="attribute.value")
        if not isinstance(value, (str, bool)):
            value = _number(value)
        output[normalized] = value
    return tuple(output.items())


@dataclass(frozen=True)
class Node:
    """An immutable authoring node; component and fragment nodes emit no wrapper."""

    tag: str | None
    children: tuple = ()
    attributes: tuple = ()
    style: Style = Style()
    part: str | None = None
    name: str | None = None
    source: tuple[str, int] | None = None
    boundary: bool = False
    declared_parts: tuple[str, ...] | None = None

    def __post_init__(self):
        if self.tag is not None and (not isinstance(self.tag, str) or not _TAG_NAME.fullmatch(self.tag)):
            raise AuthoringError("Invalid element tag", code="element.tag")
        object.__setattr__(self, "children", _children(self.children))
        object.__setattr__(self, "attributes", _attrs(dict(self.attributes)))
        object.__setattr__(self, "style", Style(self.style))
        declared = _declared_parts(self.declared_parts)
        object.__setattr__(self, "declared_parts", declared)
        if declared is not None:
            if not self.boundary:
                raise AuthoringError("Only component boundaries declare parts", code="component.parts")
            if unknown := set(_owned_parts(self.children)) - set(declared):
                raise AuthoringError(
                    f"Authored parts are missing from the component declaration: {', '.join(sorted(unknown))}",
                    code="component.parts",
                )
        for name in (self.part, self.name):
            if name is not None and (not isinstance(name, str) or not name.strip()):
                raise AuthoringError("Component and part names must be nonempty strings", code="component.name")
        if self.source is not None:
            if len(self.source) != 2 or not isinstance(self.source[0], str) or not isinstance(self.source[1], int):
                raise AuthoringError("Source locations require a filename and line number", code="component.source")
            object.__setattr__(self, "source", tuple(self.source))
        if self.tag is None and (self.attributes or self.style.entries or self.part is not None):
            raise AuthoringError("Virtual nodes cannot carry element styles or attributes", code="style.root")
        if self.tag is not None and self.tag.casefold() in _HTML_VOID_ELEMENTS and self.children:
            raise AuthoringError(f"Void element {self.tag!r} cannot have children", code="element.void")

    @property
    def parts(self) -> tuple[str, ...]:
        """Exposed names, including declared parts absent from this instance."""
        found = {}
        pending = [(self, True)]
        while pending:
            node, root = pending.pop()
            if node.boundary:
                if not root:
                    continue
                if node.declared_parts is not None:
                    found.update((name, None) for name in node.declared_parts)
                    continue
            if node.part is not None:
                found[node.part] = None
            delegated = root and node.tag is None and len(node.children) == 1
            pending.extend((child, delegated) for child in reversed(node.children) if isinstance(child, Node))
        return tuple(found)

    def with_style(self, *fragments, **properties) -> Node:
        node, wrappers = self, []
        while node.tag is None:
            if node.name is None or len(node.children) != 1 or not isinstance(node.children[0], Node):
                raise AuthoringError("Root styling requires one element; give this fragment an explicit container", code="style.root")
            wrappers.append(node)
            node = node.children[0]
        node = replace(node, style=Style(node.style, *fragments, **properties))
        for wrapper in reversed(wrappers):
            node = replace(wrapper, children=(node,))
        return node

    def with_parts(self, **overrides) -> Node:
        """Style exposed parts; declared optional parts may be absent from the tree."""
        styles = {name: Style(value) for name, value in overrides.items()}
        if missing := styles.keys() - set(self.parts):
            available = ", ".join(self.parts) or "none"
            raise AuthoringError(
                f"Unknown component parts: {', '.join(sorted(missing))}. Available parts: {available}",
                code="style.part", path=(self.name,) if self.name else (),
            )

        replacements = {}
        pending = [(self, True, False, frozenset(styles))]
        while pending:
            node, root, visited, allowed = pending.pop()
            if node.boundary and not root:
                replacements[id(node)] = node
            elif not visited:
                if node.boundary and node.declared_parts is not None:
                    allowed = allowed.intersection(node.declared_parts)
                pending.append((node, root, True, allowed))
                delegate_root = root and node.tag is None and len(node.children) == 1
                pending.extend((child, delegate_root, False, allowed) for child in reversed(node.children) if isinstance(child, Node))
            else:
                updated = node
                if node.part in allowed:
                    updated = node.with_style(styles[node.part])
                replacements[id(node)] = replace(updated, children=tuple(
                    replacements[id(child)] if isinstance(child, Node) else child for child in updated.children))
        return replacements[id(self)]


def _element(tag, children, *, base=None, style=None, class_name=None, attrs=None, part=None):
    if part is not None and (not isinstance(part, str) or not part.strip()):
        raise AuthoringError("Part names must be nonempty strings", code="style.part")
    attributes = dict(attrs or {})
    if class_name is not None:
        if not isinstance(class_name, str):
            raise AuthoringError("class_name must be a string", code="attribute.value")
        if "class" in attributes or "class_name" in attributes:
            raise AuthoringError("Supply class_name or attrs['class'], not both", code="attribute.duplicate")
        attributes["class"] = class_name
    return Node(tag, _children(children), _attrs(attributes), Style(base, style), part)


def _primitive(name, base=None):
    tag = PRIMITIVE_TAGS[name]
    def build(*children, style=None, class_name=None, attrs=None, part=None) -> Node:
        return _element(tag, children, base=base, style=style, class_name=class_name, attrs=attrs, part=part)
    build.__name__ = name
    build.__qualname__ = name
    build.__doc__ = f"Create {name} with immutable children and styles."
    return build


Box = _primitive("Box")
Paragraph = _primitive("Paragraph", style(margin=0))
Text = _primitive("Text")
Strong = _primitive("Strong")
Emphasis = _primitive("Emphasis")
Section = _primitive("Section")
Header = _primitive("Header")
Footer = _primitive("Footer")
Card = _primitive("Card", style(padding=pt(12), border="1pt solid #d3ded8", border_radius=pt(4)))


def Fragment(*children) -> Node:
    return Node(None, _children(children))


def Heading(*children, level: int = 2, keep_with_next: bool = True, style=None, class_name=None, attrs=None, part=None) -> Node:
    if isinstance(level, bool) or not isinstance(level, int) or not 1 <= level <= 6:
        raise AuthoringError("Heading level must be an integer from 1 to 6", code="heading.level")
    if not isinstance(keep_with_next, bool):
        raise AuthoringError("keep_with_next must be a boolean", code="heading.keep")
    return _element(f"h{level}", children, base=Style(margin=0, break_after="avoid" if keep_with_next else "auto"),
                    style=style, class_name=class_name, attrs=attrs, part=part)


def Stack(*children, gap=pt(8), align="stretch", style=None, class_name=None, attrs=None, part=None) -> Node:
    if align not in ALIGNMENTS:
        raise AuthoringError(f"Unknown alignment {align!r}", code="layout.align")
    return _element("div", children, base=Style(display="flex", flex_direction="column", gap=gap, align_items=align),
                    style=style, class_name=class_name, attrs=attrs, part=part)


def Row(*children, gap=pt(8), align="center", justify="space-between", style=None, class_name=None, attrs=None, part=None) -> Node:
    if align not in ALIGNMENTS:
        raise AuthoringError(f"Unknown alignment {align!r}", code="layout.align")
    if justify not in JUSTIFICATIONS:
        raise AuthoringError(f"Unknown justification {justify!r}", code="layout.justify")
    return _element("div", children, base=Style(display="flex", gap=gap, align_items=align, justify_content=justify),
                    style=style, class_name=class_name, attrs=attrs, part=part)


def Flow(*children, gap=pt(8), style=None, class_name=None, attrs=None, part=None) -> Node:
    """Ordinary block flow. Spacing is attached to blocks, without a flex wrapper."""
    captured = _children(children)
    if any(not isinstance(child, Node) or child.tag is None and child.name is None for child in captured):
        raise AuthoringError("Flow children must be rooted components; wrap text in Paragraph", code="flow.child")
    spaced = tuple(child if index == 0 else child.with_style(margin_top=gap) for index, child in enumerate(captured))
    return _element("div", spaced, style=style, class_name=class_name, attrs=attrs, part=part)


def Grid(*children, columns: int | str = 2, gap=pt(8), style=None, class_name=None, attrs=None, part=None) -> Node:
    if isinstance(columns, bool) or not isinstance(columns, (int, str)) or isinstance(columns, int) and columns < 1:
        raise AuthoringError("Grid columns must be a positive integer or explicit CSS tracks", code="grid.columns")
    tracks = " ".join(["1fr"] * columns) if isinstance(columns, int) else columns
    return _element("div", children, base=Style(display="grid", grid_template_columns=tracks, gap=gap),
                    style=style, class_name=class_name, attrs=attrs, part=part)


def Image(*, src: str, alt: str, style=None, class_name=None, attrs=None, part=None) -> Node:
    if not isinstance(src, str) or not src or not isinstance(alt, str):
        raise AuthoringError("Image requires src and an explicit alt string", code="image.required")
    return _element("img", (), style=style, class_name=class_name, attrs={**(attrs or {}), "src": src, "alt": alt}, part=part)


def PageBreak() -> Node:
    return _element("div", (), style=Style(break_before="page"), attrs={"aria-hidden": True})


class ComponentFactory(Protocol[P]):
    """A typed function component with an optional declared customization contract."""

    @property
    def parts(self) -> tuple[str, ...] | None: ...

    def __call__(self, *args: P.args, **kwargs: P.kwargs) -> Node: ...


@overload
def component(fn: Callable[P, Node], *, parts: Iterable[str] | None = None) -> ComponentFactory[P]: ...


@overload
def component(fn: None = None, *, parts: Iterable[str] | None = None) -> Callable[[Callable[P, Node]], ComponentFactory[P]]: ...


def component(
    fn: Callable[P, Node] | None = None, *, parts: Iterable[str] | None = None,
) -> ComponentFactory[P] | Callable[[Callable[P, Node]], ComponentFactory[P]]:
    """Retain component identity; optionally declare stable names for customization."""
    declared = _declared_parts(parts)
    if fn is None:
        return lambda factory: component(factory, parts=declared)

    @wraps(fn)
    def wrapped(*args: P.args, **kwargs: P.kwargs) -> Node:
        try:
            result = fn(*args, **kwargs)
            children = _children((result,))
            code = getattr(fn, "__code__", None)
            source = (code.co_filename, code.co_firstlineno) if code else None
            return Node(None, children, name=fn.__name__, source=source, boundary=True, declared_parts=declared)
        except AuthoringError as error:
            raise error.at(fn.__name__) from error
        except (TypeError, ValueError) as error:
            raise AuthoringError(str(error), code="component.call", path=(fn.__name__,)) from error
    setattr(wrapped, "parts", declared)
    return cast(ComponentFactory[P], wrapped)


def styled(factory: Callable[..., Node], *fragments, **properties):
    """A reusable root style. Call-site style overrides recipe declarations."""
    base = Style(*fragments, **properties)

    @wraps(factory)
    def build(*args, style=None, **kwargs) -> Node:
        node = factory(*args, **kwargs)
        if not isinstance(node, Node):
            raise AuthoringError("A styled factory must return a component node", code="style.root")
        return node.with_style(base, style)
    setattr(build, "parts", getattr(factory, "parts", None))
    return build
