# SPDX-License-Identifier: MIT
from __future__ import annotations

from dataclasses import replace

from .._style import AuthoringError
from .._tree import Node, _children, component
from ._core import boolean, choice, identifier, node
from ._schema import SIZES


def _validation(valid):
    if valid is not None and not isinstance(valid, bool):
        raise AuthoringError("valid must be True, False or None", code="bootstrap.validation")
    return "is-valid" if valid is True else "is-invalid" if valid is False else ""


@component
def Input(*, type="text", value=None, name=None, placeholder=None, size=None,
          readonly=False, disabled=False, plaintext=False, valid=None, **props):
    choice(type, ("text", "email", "password", "number", "tel", "url", "search", "date", "datetime-local",
                  "month", "week", "time", "file", "color", "hidden"), "input type")
    choice(size, SIZES, "input size", optional=True)
    for label, flag in (("readonly", readonly), ("disabled", disabled), ("plaintext", plaintext)):
        boolean(flag, label)
    control = node("input", (), "form-control-plaintext" if plaintext else "form-control",
                "form-control-" + size if size else "", "form-control-color" if type == "color" else "",
                _validation(valid), attributes={"type": type, "value": value, "name": name,
                    "placeholder": placeholder, "readonly": readonly, "disabled": disabled}, **props)
    printed = value if value is not None else placeholder if placeholder is not None else "\u00a0"
    if type == "password" and value is not None:
        printed = "\u2022" * len(str(value)) or "\u00a0"
    return replace(control, _print=node("div", (printed,), "fb-bs-print-input",
                                       attributes={"hidden": type == "hidden"}))


@component
def Textarea(*children, name=None, rows=None, placeholder=None, size=None, readonly=False,
             disabled=False, valid=None, **props):
    choice(size, SIZES, "textarea size", optional=True)
    if rows is not None and (isinstance(rows, bool) or not isinstance(rows, int) or rows < 1):
        raise AuthoringError("Textarea rows must be a positive integer", code="bootstrap.input")
    boolean(readonly, "readonly")
    boolean(disabled, "disabled")
    children = _children(children)
    control = node("textarea", children, "form-control", "form-control-" + size if size else "", _validation(valid),
                attributes={"name": name, "rows": rows, "placeholder": placeholder, "readonly": readonly,
                            "disabled": disabled}, **props)
    return replace(control, _print=node("div", children or (placeholder or "\u00a0",), "fb-bs-print-textarea"))


@component
def Select(*children, name=None, size=None, multiple=False, disabled=False, valid=None, **props):
    choice(size, SIZES, "select size", optional=True)
    boolean(multiple, "multiple")
    boolean(disabled, "disabled")
    children = _children(children)
    control = node("select", children, "form-select", "form-select-" + size if size else "", _validation(valid),
                attributes={"name": name, "multiple": multiple, "disabled": disabled}, **props)
    options = []
    pending = list(reversed(children))
    while pending:
        item = pending.pop()
        if isinstance(item, Node):
            if item.tag == "option":
                options.append(item)
            else:
                pending.extend(reversed(item.children))
    selected = [option for option in options if dict(option.attributes).get("selected", False) is not False]
    if not multiple:
        selected = selected[-1:] or options[:1]
    labels = []
    for option in selected:
        if labels:
            labels.append(", ")
        labels.extend((dict(option.attributes)["label"],) if "label" in dict(option.attributes) else option.children)
    return replace(control, _print=node("div", labels or ("\u00a0",), "fb-bs-print-select"))


@component
def Check(label=None, *, id, name=None, value=None, checked=False, disabled=False, inline=False,
          radio=False, switch=False, valid=None, **props):
    identifier(id)
    for label_name, flag in (("checked", checked), ("disabled", disabled), ("inline", inline), ("radio", radio), ("switch", switch)):
        boolean(flag, label_name)
    if radio and switch:
        raise AuthoringError("A switch is a checkbox, not a radio", code="bootstrap.check")
    control = node("input", (), "form-check-input", _validation(valid), attributes={
        "type": "radio" if radio else "checkbox", "id": id, "name": name, "value": value,
        "checked": checked, "disabled": disabled, "role": "switch" if switch else None,
    })
    control = replace(control, _print=node("span", ("\u00a0",), "fb-bs-print-check",
                      attributes={"data-checked": "true" if checked else "false", "role": "img",
                                  "data-kind": "switch" if switch else "radio" if radio else "checkbox",
                                  "aria-label": "Selected" if checked else "Not selected"}))
    label_node = node("label", (label,), "form-check-label", attributes={"for": id}) if label is not None else None
    result = node("div", (control, label_node), "form-check", "form-switch" if switch else "",
                  "form-check-inline" if inline else "", **props)
    from .._style import style
    return replace(result, _print=node("div", (control, node("span", (label,)) if label is not None else None),
                   "fb-bs-print-check-row", style=style(display="inline-grid" if inline else "grid",
                   grid_template_columns="2em 1fr" if switch else "1em 1fr", column_gap=".35em", padding_left=0)))


@component
def Range(*, value=None, min=None, max=None, step=None, name=None, disabled=False, **props):
    from decimal import Decimal
    import math
    from .._style import pct, style
    lower, upper = 0 if min is None else min, 100 if max is None else max
    if any(isinstance(number, bool) or not isinstance(number, (int, float, Decimal)) or not math.isfinite(number)
           for number in (lower, upper)) or lower >= upper:
        raise AuthoringError("Range requires finite min < max", code="bootstrap.range")
    actual = (float(lower) + float(upper)) / 2 if value is None else value
    if isinstance(actual, bool) or not isinstance(actual, (int, float, Decimal)) or not math.isfinite(actual) or not lower <= actual <= upper:
        raise AuthoringError("Range value must be finite and within min/max", code="bootstrap.range")
    boolean(disabled, "disabled")
    control = node("input", (), "form-range", attributes={"type": "range", "value": actual, "min": min,
                "max": max, "step": step, "name": name, "disabled": disabled}, **props)
    if step is not None and step != "any" and (isinstance(step, bool) or not isinstance(step, (int, float, Decimal))
                                                or not math.isfinite(step) or step <= 0):
        raise AuthoringError("Range step must be positive or 'any'", code="bootstrap.range")
    fraction = (float(actual) - float(lower)) / (float(upper) - float(lower)) * 100
    printed = node("div", (node("div", (node("div", (), "progress-bar", style=style(width=pct(fraction))),), "progress"),
                           node("small", (f"{actual:g} / {upper:g}",))), "fb-bs-print-range")
    return replace(control, _print=printed)


@component
def InputGroup(*children, size=None, wrap=True, **props):
    choice(size, SIZES, "input group size", optional=True)
    boolean(wrap, "wrap")
    children = _children(children)
    result = node("div", children, "input-group", "input-group-" + size if size else "",
                  "flex-nowrap" if not wrap else "", **props)
    columns = []
    for child in children:
        leaf = child
        while isinstance(leaf, Node) and leaf.tag is None and len(leaf.children) == 1:
            leaf = leaf.children[0]
        columns.append("1fr" if isinstance(leaf, Node) and leaf.tag in {"input", "textarea", "select"} else "auto")
    from .._style import style
    return replace(result, _print=node("div", children, "fb-bs-print-input-group",
                   style=style(display="grid", grid_template_columns=" ".join(columns) or "1fr")))


@component
def FloatingLabel(control, label, *, id, **props):
    """Associate a control and label using one authored id; no JavaScript needed."""
    identifier(id)
    if not isinstance(control, Node):
        raise AuthoringError("FloatingLabel needs an Input, Textarea or Select", code="bootstrap.floating")
    wrappers, leaf = [], control
    while leaf.tag is None and len(leaf.children) == 1 and isinstance(leaf.children[0], Node):
        wrappers.append(leaf)
        leaf = leaf.children[0]
    if leaf.tag not in {"input", "textarea", "select"}:
        raise AuthoringError("FloatingLabel needs an Input, Textarea or Select", code="bootstrap.floating")
    attributes = dict(leaf.attributes)
    if attributes.get("id", id) != id:
        raise AuthoringError("Floating label and control ids must agree", code="bootstrap.id")
    attributes["id"] = id
    if leaf.tag != "select" and not attributes.get("placeholder"):
        attributes["placeholder"] = " "
    leaf = replace(leaf, attributes=tuple(attributes.items()))
    for wrapper in reversed(wrappers):
        leaf = replace(wrapper, children=(leaf,))
    result = node("div", (leaf, node("label", (label,), attributes={"for": id})), "form-floating", **props)
    # A print field uses a visible label above its captured value, avoiding
    # placeholder/focus selectors and absolute browser-label positioning.
    return replace(result, _print=node("div", (node("div", (label,), "form-label"), leaf), "fb-bs-print-floating"))


@component
def Feedback(*children, valid=False, tooltip=False, **props):
    boolean(tooltip, "tooltip")
    if not isinstance(valid, bool):
        raise AuthoringError("Feedback valid must be a boolean", code="bootstrap.validation")
    return node("div", children, ("valid" if valid else "invalid") + ("-tooltip" if tooltip else "-feedback"), **props)
