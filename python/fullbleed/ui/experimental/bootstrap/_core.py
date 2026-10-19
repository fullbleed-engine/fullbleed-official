# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Iterable, Mapping, Set
from dataclasses import replace
from functools import lru_cache
from hashlib import sha256
from importlib.resources import files
import re

from .._resources import Resource
from .._style import AuthoringError
from .._tree import _element
from ._schema import CSS_SHA256, VERSION


def classes(*values) -> str:
    """Compose literal class names and conditional mappings in authored order."""
    result, active = {}, set()

    def visit(value):
        if value is None or value is False:
            return
        if isinstance(value, str):
            result.update((name, None) for name in value.split())
            return
        if isinstance(value, Mapping):
            for name, enabled in value.items():
                if not isinstance(name, str) or not isinstance(enabled, bool):
                    raise AuthoringError("Conditional classes require string names and boolean values", code="bootstrap.classes")
                if enabled:
                    visit(name)
            return
        if isinstance(value, Iterable) and not isinstance(value, (Set, bytes, bytearray)):
            identity = id(value)
            if identity in active:
                raise AuthoringError("Cyclic class collection", code="bootstrap.classes")
            active.add(identity)
            try:
                for item in value:
                    visit(item)
            finally:
                active.remove(identity)
            return
        raise AuthoringError("Classes require strings, ordered collections or conditional mappings", code="bootstrap.classes")

    visit(values)
    return " ".join(result)


@lru_cache(maxsize=1)
def stylesheet() -> str:
    """The bundled, unmodified Bootstrap 5.0.0 CSS; never fetches a URL."""
    data = files("fullbleed_assets").joinpath("bootstrap.min.css").read_bytes()
    if sha256(data).hexdigest() != CSS_SHA256:
        raise AuthoringError("The bundled Bootstrap stylesheet does not match 5.0.0", code="bootstrap.asset")
    return data.decode("utf-8")


@lru_cache(maxsize=1)
def resources():
    return (Resource("bootstrap@" + VERSION, "css", stylesheet()),
            Resource("fullbleed/bootstrap-static@1", "css", STATIC_CSS))


# Scoped adjustments turn viewport overlays and animation into authored specimens.
# Bootstrap itself remains unmodified and hash pinned. These rules also apply in
# standalone HTML; they are not a hidden PDF-only rendering mode.
STATIC_CSS = """.fb-bs-modal { height: auto; overflow: visible; }
.fb-bs-modal .modal-dialog { margin: .5rem 0; }
.fb-bs-modal .modal-dialog-centered { min-height: 0; }
.fb-bs-modal .modal-dialog-scrollable { height: auto; }
.fb-bs-offcanvas { visibility: visible; transform: none; height: auto; max-width: 100%; }
.fb-bs-spinner { animation: none; }
.fb-bs-spinner.spinner-grow { opacity: 1; transform: scale(.75); }
.fb-bs-print-input, .fb-bs-print-textarea, .fb-bs-print-select { min-height: 2.375rem; }
.fb-bs-print-textarea { white-space: pre-wrap; }
.fb-bs-print-floating.form-floating > .form-control, .fb-bs-print-floating.form-floating > .form-select { height: auto; padding: .375rem .75rem; }
.fb-bs-print-check { display: inline-block; text-align: center; font-size: .8em; line-height: 1; }
.fb-bs-print-check[data-checked=true] { background-color: #0d6efd; border-color: #0d6efd; color: white; }
.fb-bs-print-check-row .form-check-input { float: none; margin-left: 0; }
.fb-bs-print-check[data-checked=true][data-kind=checkbox] { background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath fill='none' stroke='white' stroke-linecap='round' stroke-linejoin='round' stroke-width='3' d='m4 8 3 3 5-6'/%3E%3C/svg%3E"); }
.fb-bs-print-check[data-checked=true][data-kind=radio] { border-radius: 50%; background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Ccircle cx='8' cy='8' r='4' fill='white'/%3E%3C/svg%3E"); }
.fb-bs-print-check[data-checked=true][data-kind=switch] { background-position: right center; background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Ccircle cx='8' cy='8' r='6' fill='white'/%3E%3C/svg%3E"); }
.fb-bs-print-input-group.input-group > .form-control, .fb-bs-print-input-group.input-group > .form-select { width: 100%; }
.fb-bs-print-range.form-range { height: auto; }
"""


def choice(value, allowed, label, *, optional=False):
    if value is None and optional:
        return None
    if value not in allowed:
        raise AuthoringError(f"{label} must be one of: {', '.join(map(str, allowed))}", code="bootstrap.option")
    return value


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", value):
        raise AuthoringError("Use an id starting with a letter and containing letters, digits, '-' or '_'",
                             code="bootstrap.id")
    return value


def boolean(value, label):
    if not isinstance(value, bool):
        raise AuthoringError(f"{label} must be a boolean", code="bootstrap.option")
    return value


def node(tag, children=(), *base_classes, attributes=None, _part=None, **props):
    class_name = classes(*base_classes, props.pop("class_name", None))
    style = props.pop("style", None)
    attrs = dict(attributes or {})
    extra = dict(props.pop("attrs", None) or {})
    if (id_value := props.pop("id", None)) is not None:
        if "id" in extra and extra["id"] != id_value:
            raise AuthoringError("Supply id once", code="bootstrap.id")
        extra["id"] = id_value
    if props:
        raise TypeError("Unknown component options: " + ", ".join(sorted(props)) + "; put HTML attributes in attrs=")
    for name, value in extra.items():
        if name in attrs and attrs[name] != value:
            raise AuthoringError(f"The component owns attribute {name!r}; use its named option", code="bootstrap.attribute")
        attrs[name] = value
    result = _element(tag, children, class_name=class_name or None, style=style, attrs=attrs, part=_part)
    return replace(result, _resources=resources())


def list_node(tag, children=(), *base_classes, **props):
    """Keep browser list semantics and a CSS layout box in the print tree.

    Native HTML list handling otherwise imposes list rows over Bootstrap's flex
    navigation. Roles retain the list intent in the derived static source.
    """
    result = node(tag, children, *base_classes, **props)
    return replace(result, _print=node("div", result.children,
                   attributes={"role": "listitem" if tag == "li" else "list"}))


def link_box(element, children, href=None, *, inline=False):
    """Paint the complete Bootstrap box while retaining a real print link."""
    from .._style import css, style
    content = (node("a", children, attributes={"href": href},
                    style=style(color="inherit", text_decoration="none")),) if href is not None else children
    # Static print links do not need the interactive z-index used by Bootstrap
    # for overlapping focus borders. Keep their paint order in document flow.
    return replace(element, _print=node("div", content,
                   style=style(css("position: static; z-index: auto"), display="inline-block" if inline else "block")))


def primitive(name, tag, class_name):
    def build(*children, style=None, class_name=None, attrs=None, id=None):
        factory = list_node if name in {"NavItem", "NavbarNav"} else node
        return factory(tag, children, base_class, style=style, class_name=class_name, attrs=attrs, id=id)
    base_class = class_name
    build.__name__ = build.__qualname__ = name
    build.__doc__ = f"Create a Bootstrap {name}; appearance travels with the component."
    return build
