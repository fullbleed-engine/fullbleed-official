# SPDX-License-Identifier: MIT
"""Immutable authoring values. This module performs no rendering or asset I/O."""
from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from decimal import Decimal
import difflib
import math
import re
import warnings

from ..style import StyleWarning, _parse_style_string


class AuthoringError(ValueError):
    """An authoring failure with a component path, suitable for tooling."""

    def __init__(self, message: str, *, code: str = "authoring.invalid", path=()):
        self.message, self.code, self.path = message, code, tuple(path)
        super().__init__((" → ".join(self.path) + ": " if self.path else "") + message)

    def at(self, name: str) -> AuthoringError:
        return AuthoringError(self.message, code=self.code, path=(name, *self.path))

    def to_dict(self) -> dict:
        return {"code": self.code, "message": self.message, "path": list(self.path)}


def _number(value) -> str:
    if isinstance(value, bool) or not isinstance(value, (int, float, Decimal)):
        raise AuthoringError("Expected a finite number", code="value.number")
    try:
        finite = math.isfinite(value)
    except (ValueError, OverflowError):
        finite = False
    if not finite:
        raise AuthoringError("Lengths and numeric styles must be finite", code="value.finite")
    return str(value)


@dataclass(frozen=True)
class Length:
    value: int | float | Decimal
    unit: str

    def __post_init__(self):
        _number(self.value)
        if self.unit not in {"pt", "mm", "cm", "in", "px", "%", "em", "rem"}:
            raise AuthoringError(f"Unsupported length unit: {self.unit}", code="value.unit")

    def __str__(self):
        return _number(self.value) + self.unit


def pt(value) -> Length:
    return Length(value, "pt")


def mm(value) -> Length:
    return Length(value, "mm")


def pct(value) -> Length:
    return Length(value, "%")


@dataclass(frozen=True)
class Token:
    name: str

    def __post_init__(self):
        if not isinstance(self.name, str) or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_.-]*", self.name):
            raise AuthoringError("Token names must be nonempty identifiers", code="theme.name")


def token(name: str) -> Token:
    return Token(name)


# A deliberately bounded, checked vocabulary for the experimental authoring API.
# css(...) retains explicit access to additional engine CSS declarations.
_PROPERTIES = frozenset("""
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
_LENGTHS = frozenset("""
    font-size letter-spacing gap row-gap column-gap width height min-width
    max-width min-height max-height margin margin-top margin-right margin-bottom
    margin-left padding padding-top padding-right padding-bottom padding-left
    border-width border-radius
""".split())


def _value(value):
    if isinstance(value, (Length, Token)):
        return value
    if isinstance(value, (int, float, Decimal)) and not isinstance(value, bool):
        return _number(value)
    if isinstance(value, str) and value.strip():
        return value.strip()
    if isinstance(value, (tuple, list)):
        return tuple(_value(part) for part in value)
    raise AuthoringError("Styles accept strings, finite numbers, lengths, tokens, or ordered tuples", code="style.value")


def _validate_declaration(name: str, value: str):
    with warnings.catch_warnings():
        warnings.simplefilter("error", StyleWarning)
        try:
            parsed = _parse_style_string(f"{name}: {value};")
        except StyleWarning as error:
            raise AuthoringError(str(error), code="style.syntax") from error
    if parsed != [(name, value)]:
        raise AuthoringError(f"Invalid declaration value for {name}", code="style.syntax")


@dataclass(frozen=True, init=False)
class Style:
    """Ordered declarations, snapshotted once; later declarations win by property."""

    entries: tuple

    def __init__(self, *fragments, **properties):
        result = {}
        for fragment in (*fragments, properties):
            if fragment is None:
                continue
            if not isinstance(fragment, (Style, Mapping)):
                raise AuthoringError("Compose Style values or mappings; use css(...) for CSS text", code="style.fragment")
            pairs = fragment.entries if isinstance(fragment, Style) else fragment.items()
            for name, value in pairs:
                if value is None:
                    continue
                if not isinstance(name, str):
                    raise AuthoringError("Style property names must be strings", code="style.property")
                normalized = name if name.startswith("--") else name.replace("_", "-")
                if not isinstance(fragment, Style) and normalized not in _PROPERTIES and not normalized.startswith("--"):
                    hint = difflib.get_close_matches(normalized, _PROPERTIES, n=1)
                    suffix = f"; did you mean {hint[0]!r}?" if hint else "; use css(...) for additional CSS"
                    raise AuthoringError(f"Unknown style property {name!r}{suffix}", code="style.property")
                if not re.fullmatch(r"(?:--)?[A-Za-z][A-Za-z0-9-]*", normalized):
                    raise AuthoringError(f"Invalid style property {name!r}", code="style.property")
                frozen = _value(value)
                if normalized in _LENGTHS and isinstance(value, (int, float, Decimal)) and value != 0:
                    raise AuthoringError(f"{name} needs an explicit unit, for example pt({value})", code="style.unit")
                result.pop(normalized, None)
                result[normalized] = frozen
        object.__setattr__(self, "entries", tuple(result.items()))

    def resolve(self, theme: Theme) -> str:
        def resolve(value):
            if isinstance(value, Token):
                try:
                    value = dict(theme.values)[value.name]
                except KeyError as error:
                    raise AuthoringError(f"Missing theme token {value.name!r}", code="theme.missing") from error
            if isinstance(value, tuple):
                return " ".join(resolve(part) for part in value)
            return str(value)

        declarations = []
        for name, value in self.entries:
            text = resolve(value)
            _validate_declaration(name, text)
            if name in _LENGTHS and re.fullmatch(r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)", text) and Decimal(text) != 0:
                raise AuthoringError(f"{name} needs an explicit unit", code="style.unit")
            declarations.append(f"{name}: {text};")
        return " ".join(declarations)


def style(*fragments, **properties) -> Style:
    return Style(*fragments, **properties)


def css(declarations: str) -> Style:
    """Explicit declaration escape hatch; this is not a selector stylesheet."""
    if not isinstance(declarations, str):
        raise TypeError("css expects declaration text")
    with warnings.catch_warnings():
        warnings.simplefilter("error", StyleWarning)
        try:
            entries = tuple(_parse_style_string(declarations))
        except StyleWarning as error:
            raise AuthoringError(str(error), code="style.syntax") from error
    result = object.__new__(Style)
    object.__setattr__(result, "entries", entries)
    return result


@dataclass(frozen=True, init=False)
class Theme:
    values: tuple
    text: Style

    def __init__(self, values: Mapping | None = None, *, text: Style | Mapping | None = None):
        entries = []
        for name, value in (values or {}).items():
            Token(name)
            captured = _value(value)
            if isinstance(captured, (Token, tuple)):
                raise AuthoringError("Theme tokens must be scalar values or lengths", code="theme.value")
            entries.append((name, captured))
        object.__setattr__(self, "values", tuple(entries))
        object.__setattr__(self, "text", Style(text))
