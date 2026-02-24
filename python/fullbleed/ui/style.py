from __future__ import annotations

import warnings
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any


class StyleWarning(UserWarning):
    """Warning emitted for suspicious inline-style authoring inputs."""


StyleLike = Any


def _normalize_prop_name(name: Any) -> str:
    text = str(name).strip()
    if not text:
        return text
    if text.startswith("--"):
        return text
    return text.replace("_", "-").lower()


def _warn(msg: str) -> None:
    warnings.warn(msg, StyleWarning, stacklevel=3)


def _coerce_style_value(prop: str, value: Any) -> str | None:
    if value is None or value is False:
        return None
    if isinstance(value, bool):
        _warn(f"Skipping boolean inline-style value for {prop!r}: {value!r}")
        return None
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, (list, tuple)):
        if not value:
            return None
        return " ".join(str(part) for part in value if part is not None)
    text = str(value).strip()
    if not text:
        return None
    if text in {"True", "False"}:
        _warn(f"Suspicious inline-style string value for {prop!r}: {text!r}")
    return text


def _parse_style_string(fragment: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for part in str(fragment).split(";"):
        chunk = part.strip()
        if not chunk:
            continue
        name, sep, value = chunk.partition(":")
        if not sep:
            _warn(f"Ignoring malformed inline-style fragment without ':': {chunk!r}")
            continue
        prop = _normalize_prop_name(name)
        if not prop:
            _warn(f"Ignoring inline-style fragment with empty property name: {chunk!r}")
            continue
        css_value = _coerce_style_value(prop, value)
        if css_value is None:
            continue
        out.append((prop, css_value))
    return out


@dataclass
class Style:
    """Composable inline-style declaration collection preserving insertion order."""

    _props: dict[str, str] = field(default_factory=dict)

    def merge(self, *fragments: Any, **props: Any) -> "Style":
        for fragment in fragments:
            self._merge_one(fragment)
        if props:
            self._merge_one(props)
        return self

    def _merge_one(self, fragment: Any) -> None:
        if fragment is None or fragment is False:
            return
        if isinstance(fragment, Style):
            for key, value in fragment.items():
                self._set_prop(key, value)
            return
        if isinstance(fragment, str):
            for key, value in _parse_style_string(fragment):
                self._set_prop(key, value)
            return
        if isinstance(fragment, Mapping):
            for raw_key, raw_value in fragment.items():
                key = _normalize_prop_name(raw_key)
                if not key:
                    _warn(f"Ignoring inline-style mapping entry with empty property name: {raw_key!r}")
                    continue
                value = _coerce_style_value(key, raw_value)
                if value is None:
                    continue
                self._set_prop(key, value)
            return
        if isinstance(fragment, Iterable) and not isinstance(fragment, (bytes, bytearray)):
            for item in fragment:
                self._merge_one(item)
            return
        _warn(f"Unsupported inline-style fragment type {type(fragment).__name__}; coercing to string")
        for key, value in _parse_style_string(str(fragment)):
            self._set_prop(key, value)

    def _set_prop(self, key: str, value: str) -> None:
        if key in self._props:
            # Move overridden declarations to the end to preserve authored merge order.
            self._props.pop(key, None)
        self._props[key] = value

    def items(self) -> list[tuple[str, str]]:
        return list(self._props.items())

    def to_css(self, *, trailing_semicolon: bool = True) -> str:
        if not self._props:
            return ""
        body = "; ".join(f"{name}: {value}" for name, value in self._props.items())
        return f"{body};" if trailing_semicolon else body

    @classmethod
    def from_any(cls, *fragments: Any, **props: Any) -> "Style":
        return cls().merge(*fragments, **props)


def style(*fragments: Any, **props: Any) -> Style:
    return Style.from_any(*fragments, **props)


def style_to_css(value: Any, *, trailing_semicolon: bool = True) -> str:
    if value is None or value is False:
        return ""
    if isinstance(value, str):
        return value.strip()
    return Style.from_any(value).to_css(trailing_semicolon=trailing_semicolon)


def merge_style_attr_values(existing: Any, fragment: Any) -> Any:
    """Merge style fragments for primitive helpers while preserving legacy string behavior when possible."""
    if not fragment:
        return existing
    if isinstance(existing, str) and isinstance(fragment, str):
        current = existing.strip()
        if current and not current.endswith(";"):
            current = f"{current};"
        return f"{current} {fragment}".strip()
    if existing in (None, False, ""):
        if isinstance(fragment, str):
            return fragment.strip()
        return Style.from_any(fragment)
    return Style.from_any(existing, fragment)


def _unit(value: Any, suffix: str) -> str:
    return f"{value}{suffix}"


def px(value: Any) -> str:
    return _unit(value, "px")


def pt(value: Any) -> str:
    return _unit(value, "pt")


def inch(value: Any) -> str:
    return _unit(value, "in")


def rem(value: Any) -> str:
    return _unit(value, "rem")


def pct(value: Any) -> str:
    return _unit(value, "%")
