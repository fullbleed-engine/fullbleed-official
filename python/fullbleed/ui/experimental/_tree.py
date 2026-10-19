# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Iterable, Mapping, Set
from dataclasses import dataclass, field, replace
from decimal import Decimal
from functools import wraps
from typing import Callable, ClassVar, ParamSpec, Protocol, cast, overload

from ..core import _normalize_attr_name, _ATTR_NAME, _TAG_NAME, _HTML_VOID_ELEMENTS
from ._style import AuthoringError, Style, _number, pt, style
from ._resources import Resource
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


def _collect_part_bindings(root):
    """Resolve owned markers and explicit child exports to ordered tree paths.

    Paths identify occurrences, not object identities: the same immutable child
    can appear in two positions with different public customization boundaries.
    Nested private trees are never searched for a matching spelling.
    """
    found, owned = {}, set()
    pending = [(root, (), True)]
    while pending:
        node, path, delegate = pending.pop()
        if node is not root and node.boundary:
            bindings = dict(node._part_bindings())
            if node._forwarded_parts:
                for alias, name in node._forwarded_parts:
                    owned.add(alias)
                    found.setdefault(alias, []).extend(path + target for target in bindings[name])
            elif delegate:
                for name, targets in bindings.items():
                    found.setdefault(name, []).extend(path + target for target in targets)
            continue
        if node.part is not None:
            owned.add(node.part)
            found.setdefault(node.part, []).append(path)
        delegated = delegate and node.tag is None and len(node.children) == 1
        pending.extend((child, (*path, index), delegated)
                       for index, child in reversed(tuple(enumerate(node.children))) if isinstance(child, Node))
    return tuple((name, tuple(paths)) for name, paths in found.items()), owned


def _apply_part_styles(root, targets):
    """Copy only addressed paths, iteratively and without shared-node aliasing."""
    trie = {}
    for path, declarations in targets.items():
        branch = trie
        for index in path:
            branch = branch.setdefault(index, {})
        branch[None] = declarations
    pending, results = [(root, trie, False)], []
    while pending:
        node, branch, visited = pending.pop()
        indices = tuple(index for index in branch if index is not None)
        if not visited:
            pending.append((node, branch, True))
            pending.extend((node.children[index], branch[index], False) for index in reversed(indices))
        else:
            children = list(node.children)
            for index in reversed(indices):
                children[index] = results.pop()
            results.append(replace(node, children=tuple(children), style=Style(node.style, branch.get(None))))
    return results[0]


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
            from ._page import PageNumbers
            if isinstance(value, PageNumbers):
                raise AuthoringError("Place PageNumbers directly in a PageBand left, center or right slot; "
                                     "page values are not ordinary body content.", code="page_numbers.slot")
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


def _required_tokens(children):
    names, visited = set(), set()
    pending = list(children)
    while pending:
        node = pending.pop()
        if not isinstance(node, Node) or id(node) in visited:
            continue
        visited.add(id(node))
        names.update(node._effective_style()._token_names())
        pending.extend(node.children)
        if node._print is not None:
            pending.append(node._print)
    return tuple(sorted(names))


@dataclass(frozen=True)
class Node:
    """An immutable authoring node; component and fragment nodes emit no wrapper."""

    __fullbleed_authoring_node__: ClassVar[bool] = True
    tag: str | None
    children: tuple = ()
    attributes: tuple = ()
    style: Style = Style()
    part: str | None = None
    name: str | None = None
    source: tuple[str, int] | None = None
    boundary: bool = False
    declared_parts: tuple[str, ...] | None = None
    optional_parts: tuple[str, ...] = field(default=(), kw_only=True)
    _defaults: Style = field(default=Style(), repr=False, kw_only=True)
    _resources: tuple[Resource, ...] = field(default=(), repr=False, kw_only=True)
    _print: Node | None = field(default=None, repr=False, kw_only=True)
    _forwarded_parts: tuple[tuple[str, str], ...] = field(default=(), repr=False, kw_only=True)
    _bindings: tuple | None = field(default=None, init=False, repr=False, compare=False)

    def __post_init__(self):
        if self.tag is not None and (not isinstance(self.tag, str) or not _TAG_NAME.fullmatch(self.tag)):
            raise AuthoringError("Invalid element tag", code="element.tag")
        object.__setattr__(self, "children", _children(self.children))
        object.__setattr__(self, "attributes", _attrs(dict(self.attributes)))
        object.__setattr__(self, "style", Style(self.style))
        object.__setattr__(self, "_defaults", Style(self._defaults))
        resources = tuple(self._resources)
        if any(not isinstance(resource, Resource) for resource in resources):
            raise AuthoringError("Invalid component resource", code="resource.type")
        object.__setattr__(self, "_resources", resources)
        if self._print is not None and (not isinstance(self._print, Node) or self.tag is None
                                       or self._print.tag is None or self._print._print is not None):
            raise AuthoringError("A print representation requires a concrete element snapshot", code="print.node")
        declared = _declared_parts(self.declared_parts)
        object.__setattr__(self, "declared_parts", declared)
        optional = _declared_parts(self.optional_parts)
        if optional is None or optional and (declared is None or set(optional) - set(declared)):
            raise AuthoringError("Optional parts must belong to the component declaration", code="component.parts")
        object.__setattr__(self, "optional_parts", optional)
        if declared is not None:
            if not self.boundary:
                raise AuthoringError("Only component boundaries declare parts", code="component.parts")
        for name in (self.part, self.name):
            if name is not None and (not isinstance(name, str) or not name.strip()):
                raise AuthoringError("Component and part names must be nonempty strings", code="component.name")
        if self.source is not None:
            if len(self.source) != 2 or not isinstance(self.source[0], str) or not isinstance(self.source[1], int):
                raise AuthoringError("Source locations require a filename and line number", code="component.source")
            object.__setattr__(self, "source", tuple(self.source))
        if self.tag is None and (self.attributes or self.style.entries or self._defaults.entries or self.part is not None):
            raise AuthoringError("Virtual nodes cannot carry element styles or attributes", code="style.root")
        if self.tag is not None and self.tag.casefold() in _HTML_VOID_ELEMENTS and self.children:
            raise AuthoringError(f"Void element {self.tag!r} cannot have children", code="element.void")
        if self.boundary:
            bindings, owned = _collect_part_bindings(self)
            resolved = dict(bindings)
            if declared is not None:
                if unknown := owned - set(declared):
                    raise AuthoringError(
                        f"Authored parts are missing from the component declaration: {', '.join(sorted(unknown))}",
                        code="component.parts",
                    )
                for name in declared:
                    if not resolved.get(name) and name not in optional:
                        raise AuthoringError(
                            f"Required part {name!r} has no target. Mark an owned element with part={name!r}, "
                            "forward a child's advertised part with forward_parts(), or declare an optional part.",
                            code="component.part_missing", path=(name,),
                        )
                bindings = tuple((name, resolved.get(name, ())) for name in declared)
            object.__setattr__(self, "_bindings", bindings)
        forwarded = tuple(self._forwarded_parts)
        if forwarded:
            if not self.boundary or any(not isinstance(pair, tuple) or len(pair) != 2
                                        or not all(isinstance(value, str) for value in pair) for pair in forwarded):
                raise AuthoringError("Forward parts on a component boundary using name pairs", code="component.part_forward")
            aliases = _declared_parts(alias for alias, _ in forwarded)
            if unknown := {name for _, name in forwarded} - set(self.parts):
                raise AuthoringError(
                    f"Cannot forward unknown child parts: {', '.join(sorted(unknown))}. "
                    f"Available parts: {', '.join(self.parts) or 'none'}", code="component.part_forward",
                    path=(self.name,) if self.name else (),
                )
            forwarded = tuple(zip(aliases, (name for _, name in forwarded)))
        object.__setattr__(self, "_forwarded_parts", forwarded)

    @property
    def required_tokens(self) -> tuple[str, ...]:
        """Sorted explicit theme tokens in this subtree's effective style layers.

        Includes nested components and present optional parts. It does not
        resolve values, run callbacks or infer tokens from CSS var() strings.
        """
        return _required_tokens((self,))

    @property
    def parts(self) -> tuple[str, ...]:
        """Exposed names, including declared parts absent from this instance."""
        return tuple(name for name, _ in self._part_bindings())

    def _part_bindings(self):
        return self._bindings if self._bindings is not None else _collect_part_bindings(self)[0]

    def forward_parts(self, *names: str, **aliases: str) -> Node:
        """Expose selected child parts to the containing component, without markup.

        Positional names keep their spelling; aliases map a parent's public name
        to this child's advertised name. A second call replaces the forwarding
        selection. This node's own public parts remain unchanged.
        """
        if not self.boundary:
            raise AuthoringError("Forward parts on a component; use part= on an owned element", code="component.part_forward")
        if not names and not aliases:
            raise AuthoringError("Select at least one advertised child part to forward", code="component.part_forward")
        _declared_parts((*names, *aliases))
        return replace(self, _forwarded_parts=tuple((name, name) for name in names) + tuple(aliases.items()))

    def with_style(self, *fragments, **properties) -> Node:
        return self._with_root_styles(authored=Style(*fragments, **properties))

    def to_html(self, *, include_styles: bool | None = None,
                theme=None, a11y_mode: str | None = None) -> str:
        """Export a fragment; True embeds CSS, False omits all style declarations."""
        from ._document import to_html
        return to_html(self, include_styles=include_styles,
                       theme=theme, a11y_mode=a11y_mode)

    def _effective_style(self) -> Style:
        return Style(self._defaults, self.style)

    def _for_print(self) -> Node:
        if self._print is None:
            return self
        alternative = self._print
        # Keep authored identity, selectors, ARIA and subsequent root styling.
        # Browser-only form state is already captured in the alternative content.
        omit = {"type", "value", "name", "placeholder", "checked", "disabled", "readonly", "multiple",
                "rows", "min", "max", "step", "autocomplete", "required", "for", "role", "href"}
        attributes = {key: value for key, value in self.attributes if key not in omit}
        extras = dict(alternative.attributes)
        if "class" in extras:
            extras["class"] = " ".join(dict.fromkeys((str(attributes.get("class", "")) + " " + extras["class"]).split()))
        attributes.update(extras)
        return replace(alternative, attributes=tuple(attributes.items()), style=Style(alternative.style, self.style),
                       _defaults=Style(alternative._defaults, self._defaults), name=self.name, source=self.source,
                       part=self.part, _resources=(*self._resources, *alternative._resources))

    def _remaining_defaults(self) -> Style:
        # An explicit property replaces a recipe property before token resolution.
        # Keep shorthand/longhand interactions for the native cascade to resolve.
        authored = {name for name, _ in self.style.entries}
        result = object.__new__(Style)
        object.__setattr__(result, "entries", tuple(
            (name, value) for name, value in self._defaults.entries if name not in authored))
        return result

    def _with_root_styles(self, *, defaults=None, authored=None) -> Node:
        node, wrappers = self, []
        while node.tag is None:
            if node.name is None or len(node.children) != 1 or not isinstance(node.children[0], Node):
                raise AuthoringError("Root styling requires one element; give this fragment an explicit container", code="style.root")
            wrappers.append(node)
            node = node.children[0]
        node = replace(node, _defaults=Style(node._defaults, defaults), style=Style(node.style, authored))
        for wrapper in reversed(wrappers):
            node = replace(wrapper, children=(node,))
        return node

    def with_parts(self, **overrides) -> Node:
        """Style exposed parts; declared optional parts may be absent from the tree."""
        styles = {name: Style(value) for name, value in overrides.items()}
        bindings = dict(self._part_bindings())
        if missing := styles.keys() - bindings.keys():
            available = ", ".join(bindings) or "none"
            raise AuthoringError(
                f"Unknown component parts: {', '.join(sorted(missing))}. Available parts: {available}",
                code="style.part", path=(self.name,) if self.name else (),
            )

        targets = {}
        for name, declarations in styles.items():
            for path in bindings[name]:
                targets[path] = Style(targets.get(path), declarations)
        return _apply_part_styles(self, targets) if targets else self


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
    return Node(tag, _children(children), _attrs(attributes), Style(style), part, _defaults=Style(base))


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


def Heading(*children, level: int = 2, keep_with_next: bool | None = None, style=None, class_name=None, attrs=None, part=None) -> Node:
    """A semantic heading, kept with following content by default.

    None leaves the keep policy to reusable styles or the built-in default.
    An explicit boolean overrides reusable defaults; style= overrides it.
    """
    if isinstance(level, bool) or not isinstance(level, int) or not 1 <= level <= 6:
        raise AuthoringError("Heading level must be an integer from 1 to 6", code="heading.level")
    if keep_with_next is not None and not isinstance(keep_with_next, bool):
        raise AuthoringError("keep_with_next must be a boolean or None", code="heading.keep")
    keep_style = None if keep_with_next is None else {"break_after": "avoid" if keep_with_next else "auto"}
    return _element(f"h{level}", children, base=Style(margin=0, break_after="avoid"),
                    style=Style(keep_style, style), class_name=class_name, attrs=attrs, part=part)


def Stack(*children, gap=None, align=None, style=None, class_name=None, attrs=None, part=None) -> Node:
    """A local flex column; default gap is 8 pt and alignment is stretch.

    None leaves a layout choice to reusable styles or the built-in default.
    """
    if align is not None and align not in ALIGNMENTS:
        raise AuthoringError(f"Unknown alignment {align!r}", code="layout.align")
    return _element("div", children, base=Style(display="flex", flex_direction="column", gap=pt(8), align_items="stretch"),
                    style=Style({"gap": gap, "align_items": align}, style), class_name=class_name, attrs=attrs, part=part)


def Row(*children, gap=None, align=None, justify=None, style=None, class_name=None, attrs=None, part=None) -> Node:
    """A flex row; defaults are 8 pt gap, centered alignment and space-between.

    None leaves a layout choice to reusable styles or the built-in default.
    """
    if align is not None and align not in ALIGNMENTS:
        raise AuthoringError(f"Unknown alignment {align!r}", code="layout.align")
    if justify is not None and justify not in JUSTIFICATIONS:
        raise AuthoringError(f"Unknown justification {justify!r}", code="layout.justify")
    return _element("div", children, base=Style(display="flex", gap=pt(8), align_items="center", justify_content="space-between"),
                    style=Style({"gap": gap, "align_items": align, "justify_content": justify}, style),
                    class_name=class_name, attrs=attrs, part=part)


def Flow(*children, gap=None, style=None, class_name=None, attrs=None, part=None) -> Node:
    """Native paginated block flow with independent gaps and child margins.

    Fragments contribute siblings while retaining their diagnostic boundaries.
    No gap is inserted at the beginning or end of a page fragment.
    None leaves the gap to reusable styles or the built-in 8 pt default.
    """
    captured = _children(children)
    pending = [(child, ()) for child in reversed(captured)]
    while pending:
        child, path = pending.pop()
        if not isinstance(child, Node):
            raise AuthoringError("Flow children must be blocks or fragments; wrap text in Paragraph", code="flow.child", path=path)
        if child.tag is None:
            path = (*path, child.name) if child.name else path
            pending.extend((nested, path) for nested in reversed(child.children))
    attributes = dict(_attrs(attrs))
    if "data-fb-flow" in attributes:
        raise AuthoringError("Flow owns the data-fb-flow layout annotation", code="attribute.duplicate")
    attributes["data-fb-flow"] = "true"
    return _element("div", captured, base=Style(display="block", row_gap=pt(8)),
                    style=Style({"row_gap": gap}, style), class_name=class_name, attrs=attributes, part=part)


def Grid(*children, columns: int | str | None = None, gap=None, style=None, class_name=None, attrs=None, part=None) -> Node:
    """A grid with two equal columns and an 8 pt gap by default.

    None leaves a layout choice to reusable styles or the built-in default.
    """
    if columns is not None and (isinstance(columns, bool) or not isinstance(columns, (int, str)) or isinstance(columns, int) and columns < 1):
        raise AuthoringError("Grid columns must be a positive integer, explicit CSS tracks, or None", code="grid.columns")
    tracks = " ".join(["1fr"] * columns) if isinstance(columns, int) else columns
    return _element("div", children, base=Style(display="grid", grid_template_columns="1fr 1fr", gap=pt(8)),
                    style=Style({"grid_template_columns": tracks, "gap": gap}, style),
                    class_name=class_name, attrs=attrs, part=part)


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

    @property
    def optional_parts(self) -> tuple[str, ...] | None: ...

    def __call__(self, *args: P.args, **kwargs: P.kwargs) -> Node: ...


@overload
def component(fn: Callable[P, Node], *, parts: Iterable[str] | None = None,
              optional_parts: Iterable[str] = ()) -> ComponentFactory[P]: ...


@overload
def component(fn: None = None, *, parts: Iterable[str] | None = None,
              optional_parts: Iterable[str] = ()) -> Callable[[Callable[P, Node]], ComponentFactory[P]]: ...


def component(
    fn: Callable[P, Node] | None = None, *, parts: Iterable[str] | None = None,
    optional_parts: Iterable[str] = (),
) -> ComponentFactory[P] | Callable[[Callable[P, Node]], ComponentFactory[P]]:
    """Declare required and optional customization targets at a virtual boundary."""
    required, optional = _declared_parts(parts), _declared_parts(optional_parts)
    if optional is None:
        raise AuthoringError("optional_parts requires an ordered iterable of names", code="component.parts")
    declared = _declared_parts((*(required or ()), *optional)) if required is not None or optional else None
    if fn is None:
        return lambda factory: component(factory, parts=required, optional_parts=optional)

    @wraps(fn)
    def wrapped(*args: P.args, **kwargs: P.kwargs) -> Node:
        try:
            result = fn(*args, **kwargs)
            children = _children((result,))
            code = getattr(fn, "__code__", None)
            source = (code.co_filename, code.co_firstlineno) if code else None
            return Node(None, children, name=fn.__name__, source=source, boundary=True,
                        declared_parts=declared, optional_parts=optional)
        except AuthoringError as error:
            raise error.at(fn.__name__) from error
        except (TypeError, ValueError) as error:
            raise AuthoringError(str(error), code="component.call", path=(fn.__name__,)) from error
    setattr(wrapped, "parts", declared)
    setattr(wrapped, "optional_parts", optional if declared is not None else None)
    return cast(ComponentFactory[P], wrapped)


def styled(factory: Callable[P, Node], *fragments, **properties) -> ComponentFactory[P]:
    """Supply root defaults while preserving the factory's arguments unchanged.

    Primitive defaults come first, then styled defaults from inner to outer.
    Explicit node styles win; with_style on the returned node overrides them.
    """
    base = Style(*fragments, **properties)

    @wraps(factory)
    def build(*args: P.args, **kwargs: P.kwargs) -> Node:
        node = factory(*args, **kwargs)
        if not isinstance(node, Node):
            raise AuthoringError("A styled factory must return a component node", code="style.root")
        return node._with_root_styles(defaults=base)
    setattr(build, "parts", getattr(factory, "parts", None))
    setattr(build, "optional_parts", getattr(factory, "optional_parts", None))
    return cast(ComponentFactory[P], build)
