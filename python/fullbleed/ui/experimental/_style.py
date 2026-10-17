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
from ._schema import STYLE_PROPERTIES as _PROPERTIES, LENGTH_PROPERTIES as _LENGTHS

_UNITLESS_NUMBER = re.compile(r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?")


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


def _check_length_numbers(name, value):
    if isinstance(value, (tuple, list)):
        for part in value:
            _check_length_numbers(name, part)
    elif isinstance(value, (int, float, Decimal)) and value != 0:
        raise AuthoringError(f"{name} needs an explicit unit, for example pt({value})", code="style.unit")


@dataclass(frozen=True, init=False)
class Style:
    """Ordered declarations, snapshotted once; later declarations win by property."""

    entries: tuple

    def __init__(self, *fragments, **properties):
        result = {}
        for fragment in (*fragments, properties):
            if fragment is None or fragment is False:
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
                if normalized in _LENGTHS:
                    _check_length_numbers(name, value)
                result.pop(normalized, None)
                result[normalized] = frozen
        object.__setattr__(self, "entries", tuple(result.items()))

    def _token_names(self):
        pending = [value for _, value in self.entries]
        while pending:
            value = pending.pop()
            if isinstance(value, Token):
                yield value.name
            elif isinstance(value, tuple):
                pending.extend(value)

    def resolve(self, theme: Theme) -> str:
        def resolve(value, name):
            if isinstance(value, Token):
                try:
                    value = dict(theme.values)[value.name]
                except KeyError as error:
                    raise AuthoringError(f"Missing theme token {value.name!r}", code="theme.missing") from error
            if isinstance(value, tuple):
                return " ".join(resolve(part, name) for part in value)
            text = str(value)
            if name in _LENGTHS and _UNITLESS_NUMBER.fullmatch(text) and Decimal(text) != 0:
                raise AuthoringError(f"{name} needs an explicit unit", code="style.unit")
            return text

        declarations = []
        for name, value in self.entries:
            text = resolve(value, name)
            _validate_declaration(name, text)
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
        if values is not None and not isinstance(values, Mapping):
            raise AuthoringError("Theme values must be a mapping of token names to values", code="theme.values")
        entries = []
        for name, value in (values or {}).items():
            Token(name)
            captured = _value(value)
            if isinstance(captured, (Token, tuple)):
                raise AuthoringError("Theme tokens must be scalar values or lengths", code="theme.value")
            entries.append((name, captured))
        object.__setattr__(self, "values", tuple(entries))
        object.__setattr__(self, "text", Style(text))

    def with_values(self, /, *mappings, **values) -> Theme:
        """Derive a theme without changing this theme or its text style.

        Later mappings override earlier values; keyword values apply last.
        Names and values use the same validation as Theme construction.
        """
        merged = dict(self.values)
        for mapping in mappings:
            if not isinstance(mapping, Mapping):
                raise AuthoringError("Theme updates must be mappings", code="theme.values")
            merged.update(mapping)
        merged.update(values)
        return Theme(merged, text=self.text)
