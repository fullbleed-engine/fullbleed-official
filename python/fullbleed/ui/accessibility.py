from __future__ import annotations

import warnings
from dataclasses import dataclass
from typing import Any, Iterable

from .core import DocumentArtifact, Element, component, el
from .primitives import (
    LayoutGrid,
    TBody,
    THead,
    Table,
    Td,
    Th,
    Tr,
    _apply_class,
)


SIGNATURE_STATUS_VALUES = {
    "missing",
    "present",
    "captured",
    "on_file",
    "not_required",
    "declined",
    "unknown",
}

SIGNATURE_METHOD_VALUES = {
    "drawn_electronic",
    "typed_name",
    "scanned_handwritten",
    "wet_ink_scan",
    "digital_certificate",
    "click_to_sign",
    "stamp_seal",
    "signature_line_only",
    "other",
    "unknown",
}


class A11yValidationError(ValueError):
    def __init__(self, message: str, report: dict[str, Any]) -> None:
        super().__init__(message)
        self.report = report


class A11yWarning(UserWarning):
    pass


@dataclass(frozen=True)
class A11yId:
    value: str

    def __post_init__(self) -> None:
        text = str(self.value).strip()
        if not text:
            raise ValueError("A11yId value must not be empty")
        object.__setattr__(self, "value", text)

    def __str__(self) -> str:
        return self.value


class A11yAttrs:
    @staticmethod
    def id(value: str | A11yId) -> dict[str, str]:
        return {"id": str(value)}

    @staticmethod
    def labelledby(*ids: str | A11yId) -> dict[str, str]:
        tokens = _join_idrefs(ids)
        return {"aria_labelledby": tokens} if tokens else {}

    @staticmethod
    def describedby(*ids: str | A11yId) -> dict[str, str]:
        tokens = _join_idrefs(ids)
        return {"aria_describedby": tokens} if tokens else {}

    @staticmethod
    def label(text: Any) -> dict[str, str]:
        return {"aria_label": str(text)}

    @staticmethod
    def merge(*parts: dict[str, Any] | None) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for part in parts:
            if not part:
                continue
            out.update(part)
        return out


def _join_idrefs(values: Iterable[str | A11yId]) -> str:
    tokens = [str(v).strip() for v in values if str(v).strip()]
    return " ".join(tokens)


def _normalize_tag(tag: str) -> str:
    return str(tag).strip().lower()


def _clone_element(node: Element, **patch_props: Any) -> Element:
    props = dict(node.props)
    props.update({k: v for k, v in patch_props.items() if v is not None})
    return Element(tag=node.tag, props=props, children=list(node.children))


def _text_content(node: Any) -> str:
    if node is None:
        return ""
    if isinstance(node, str):
        return node
    if isinstance(node, Element):
        return "".join(_text_content(child) for child in node.children)
    if isinstance(node, (list, tuple)):
        return "".join(_text_content(child) for child in node)
    return str(node)


def _prop_get(props: dict[str, Any], *names: str) -> Any:
    for name in names:
        if name in props:
            return props[name]
        hy = name.replace("_", "-")
        us = name.replace("-", "_")
        if hy in props:
            return props[hy]
        if us in props:
            return props[us]
    return None


def _is_trueish(value: Any) -> bool:
    if value is True:
        return True
    if value is None:
        return False
    text = str(value).strip().lower()
    return text in {"1", "true", "yes", "on"}


def _is_blank(value: Any) -> bool:
    return value is None or str(value).strip() == ""


def _id_tokens(value: Any) -> list[str]:
    if value is None:
        return []
    return [tok for tok in str(value).split() if tok.strip()]


def _diagnostic(code: str, severity: str, message: str, path: str, **extra: Any) -> dict[str, Any]:
    out = {
        "code": code,
        "severity": severity,
        "message": message,
        "path": path,
    }
    out.update(extra)
    return out


def _walk_elements(node_or_document: Any) -> tuple[list[tuple[Element, str]], dict[str, Any]]:
    meta: dict[str, Any] = {"document_title": None}
    root: Any = node_or_document
    if isinstance(node_or_document, DocumentArtifact):
        meta["document_title"] = node_or_document.title
        root = node_or_document.root
    nodes: list[tuple[Element, str]] = []

    def visit(node: Any, path: str) -> None:
        if isinstance(node, Element):
            nodes.append((node, path))
            idx = 0
            for child in node.children:
                if isinstance(child, Element):
                    idx += 1
                    visit(child, f"{path}/{_normalize_tag(child.tag)}[{idx}]")
        elif isinstance(node, (list, tuple)):
            for item in node:
                visit(item, path)

    if isinstance(root, Element):
        visit(root, f"/{_normalize_tag(root.tag)}[1]")
    elif isinstance(root, (list, tuple)):
        visit(root, "/fragment")
    return nodes, meta


class A11yContract:
    """Lightweight structural validator for authored accessibility semantics."""

    def validate(self, node_or_document: Any, *, mode: str | None = "warn") -> dict[str, Any]:
        normalized_mode = None if mode is None else str(mode).strip().lower()
        if normalized_mode not in {None, "", "warn", "raise"}:
            raise ValueError(f"Unsupported a11y validation mode {mode!r}")
        if normalized_mode == "":
            normalized_mode = None

        diagnostics: list[dict[str, Any]] = []
        nodes, meta = _walk_elements(node_or_document)

        if isinstance(node_or_document, DocumentArtifact) and _is_blank(meta.get("document_title")):
            diagnostics.append(
                _diagnostic(
                    "DOCUMENT_TITLE_MISSING",
                    "error",
                    "Document title must be present.",
                    "/document",
                )
            )

        ids: dict[str, str] = {}
        references: list[tuple[str, str, str]] = []
        main_paths: list[str] = []

        for node, path in nodes:
            tag = _normalize_tag(node.tag)
            props = node.props

            node_id = _prop_get(props, "id")
            if node_id is not None:
                text_id = str(node_id).strip()
                if not text_id:
                    diagnostics.append(
                        _diagnostic("ID_EMPTY", "error", "Element id must not be empty.", path)
                    )
                elif text_id in ids:
                    diagnostics.append(
                        _diagnostic(
                            "ID_DUPLICATE",
                            "error",
                            f"Duplicate id {text_id!r}.",
                            path,
                            id=text_id,
                            first_seen_path=ids[text_id],
                        )
                    )
                else:
                    ids[text_id] = path

            for attr_name in ("aria_labelledby", "aria_describedby"):
                for token in _id_tokens(_prop_get(props, attr_name, attr_name.replace("_", "-"))):
                    references.append((attr_name.replace("_", "-"), token, path))

            aria_label = _prop_get(props, "aria_label", "aria-label")
            if aria_label is not None and _is_blank(aria_label):
                diagnostics.append(
                    _diagnostic(
                        "ARIA_LABEL_EMPTY",
                        "error",
                        "aria-label must not be empty.",
                        path,
                    )
                )

            if tag == "main":
                main_paths.append(path)

            if tag in {"h1", "h2", "h3", "h4", "h5", "h6"} and _is_blank(_text_content(node)):
                diagnostics.append(
                    _diagnostic(
                        "HEADING_EMPTY",
                        "warning",
                        "Heading text is empty.",
                        path,
                    )
                )
            if tag == "label" and _is_blank(_text_content(node)):
                diagnostics.append(
                    _diagnostic(
                        "LABEL_EMPTY",
                        "warning",
                        "Label text is empty.",
                        path,
                    )
                )

            role = str(_prop_get(props, "role") or "").strip().lower()
            if role == "region" and _is_blank(aria_label) and not _id_tokens(
                _prop_get(props, "aria_labelledby", "aria-labelledby")
            ):
                diagnostics.append(
                    _diagnostic(
                        "REGION_UNLABELED",
                        "warning",
                        "Region landmarks should be labeled with aria-label or aria-labelledby.",
                        path,
                    )
                )

            if tag in {"img", "svg"}:
                diagnostics.extend(self._validate_image_semantics(node, path))

            sig_status = _prop_get(props, "data_fb_a11y_signature_status", "data-fb-a11y-signature-status")
            if sig_status is not None:
                sig_status_text = str(sig_status).strip()
                if sig_status_text not in SIGNATURE_STATUS_VALUES:
                    diagnostics.append(
                        _diagnostic(
                            "SIGNATURE_STATUS_INVALID",
                            "error",
                            f"Invalid signature status {sig_status_text!r}.",
                            path,
                            value=sig_status_text,
                        )
                    )
            sig_method = _prop_get(props, "data_fb_a11y_signature_method", "data-fb-a11y-signature-method")
            if sig_method is not None:
                sig_method_text = str(sig_method).strip()
                if sig_method_text not in SIGNATURE_METHOD_VALUES:
                    diagnostics.append(
                        _diagnostic(
                            "SIGNATURE_METHOD_INVALID",
                            "error",
                            f"Invalid signature method {sig_method_text!r}.",
                            path,
                            value=sig_method_text,
                        )
                    )

        for attr_name, target_id, path in references:
            if target_id not in ids:
                diagnostics.append(
                    _diagnostic(
                        "IDREF_MISSING",
                        "error",
                        f"{attr_name} references missing id {target_id!r}.",
                        path,
                        attr=attr_name,
                        target_id=target_id,
                    )
                )

        if len(main_paths) > 1:
            diagnostics.append(
                _diagnostic(
                    "MAIN_MULTIPLE",
                    "error",
                    "Only one <main> landmark should be present.",
                    "/document",
                    paths=main_paths,
                )
            )

        errors = [d for d in diagnostics if d["severity"] == "error"]
        warnings_only = [d for d in diagnostics if d["severity"] != "error"]
        report = {
            "ok": not errors,
            "mode": normalized_mode,
            "error_count": len(errors),
            "warning_count": len(warnings_only),
            "errors": errors,
            "warnings": warnings_only,
            "diagnostics": diagnostics,
        }

        if normalized_mode == "warn":
            for diag in diagnostics:
                warnings.warn(
                    f"[{diag['severity']}] {diag['code']}: {diag['message']} ({diag['path']})",
                    A11yWarning,
                    stacklevel=2,
                )
        if normalized_mode == "raise" and errors:
            raise A11yValidationError("Accessibility validation failed", report)
        return report

    def _validate_image_semantics(self, node: Element, path: str) -> list[dict[str, Any]]:
        props = node.props
        role = str(_prop_get(props, "role") or "").strip().lower()
        aria_hidden = _is_trueish(_prop_get(props, "aria_hidden", "aria-hidden"))
        explicit_decorative = _is_trueish(
            _prop_get(props, "data_fb_a11y_decorative", "data-fb-a11y-decorative")
        )
        aria_label = _prop_get(props, "aria_label", "aria-label")
        aria_labelledby = _prop_get(props, "aria_labelledby", "aria-labelledby")
        alt_value = _prop_get(props, "alt")
        title_value = _prop_get(props, "title")

        has_informative_name = bool(
            (aria_label is not None and str(aria_label).strip())
            or _id_tokens(aria_labelledby)
            or (alt_value is not None and str(alt_value).strip())
        )
        alt_empty = alt_value is not None and str(alt_value) == ""
        role_decorative = role in {"presentation", "none"}
        decorative = explicit_decorative or aria_hidden or role_decorative or alt_empty

        out: list[dict[str, Any]] = []
        if decorative and has_informative_name:
            out.append(
                _diagnostic(
                    "IMAGE_SEMANTIC_CONFLICT",
                    "error",
                    "Image has conflicting decorative and informative signals.",
                    path,
                )
            )
        if not decorative and not has_informative_name:
            if title_value is not None and str(title_value).strip():
                out.append(
                    _diagnostic(
                        "IMAGE_ALT_MISSING_TITLE_PRESENT",
                        "warning",
                        "Image has title but no alt/aria label; title is not a substitute for text alternatives.",
                        path,
                    )
                )
            else:
                out.append(
                    _diagnostic(
                        "IMAGE_ALT_MISSING",
                        "error",
                        "Informative image requires a text alternative (alt, aria-label, or aria-labelledby).",
                        path,
                    )
                )
        return out


def validate_a11y(node_or_document: Any, *, mode: str | None = "warn") -> dict[str, Any]:
    return A11yContract().validate(node_or_document, mode=mode)


@component
def Dl(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-dl", class_name)
    return el("dl", list(children), **props)


@component
def Dt(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-dt", class_name)
    return el("dt", content, **props)


@component
def Dd(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-dd", class_name)
    return el("dd", content, **props)


DefinitionList = Dl
DefinitionTerm = Dt
DefinitionDescription = Dd


@component
def FieldItem(
    label: Any,
    value: Any,
    *,
    term_class: str | None = None,
    description_class: str | None = None,
    **_props: Any,
) -> object:
    # Returns dt+dd nodes as a flattened pair for use inside FieldGrid/Dl.
    return [
        Dt(label, class_name=term_class),
        Dd(value, class_name=description_class),
    ]


@component
def FieldGrid(*children: Any, class_name: str | None = None, **props: Any) -> object:
    props.setdefault("data_fb_a11y_field_grid", "true")
    flat: list[Any] = []
    for child in children:
        if child is None:
            continue
        if isinstance(child, (list, tuple)):
            for item in child:
                if item is None:
                    continue
                if isinstance(item, (list, tuple)):
                    flat.extend(x for x in item if x is not None)
                else:
                    flat.append(item)
        else:
            flat.append(child)
    return Dl(*flat, class_name=("ui-field-grid" if class_name is None else f"ui-field-grid {class_name}"), **props)


@component
def Figure(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-figure", class_name)
    return el("figure", list(children), **props)


@component
def FigCaption(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-figcaption", class_name)
    return el("figcaption", content, **props)


@component
def TableCaption(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-table-caption", class_name)
    return el("caption", content, **props)


@component
def SemanticTable(*children: Any, class_name: str | None = None, caption: Any = None, **props: Any) -> object:
    if caption is not None:
        children = (TableCaption(caption), *children)
    return Table(*children, class_name=("ui-semantic-table" if class_name is None else f"ui-semantic-table {class_name}"), **props)


@component
def SemanticTableHead(*rows: Any, class_name: str | None = None, **props: Any) -> object:
    return THead(*rows, class_name=("ui-semantic-thead" if class_name is None else f"ui-semantic-thead {class_name}"), **props)


@component
def SemanticTableBody(*rows: Any, class_name: str | None = None, **props: Any) -> object:
    return TBody(*rows, class_name=("ui-semantic-tbody" if class_name is None else f"ui-semantic-tbody {class_name}"), **props)


@component
def SemanticTableFoot(*rows: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-semantic-tfoot", class_name)
    return el("tfoot", list(rows), **props)


@component
def SemanticTableRow(*cells: Any, class_name: str | None = None, **props: Any) -> object:
    return Tr(*cells, class_name=("ui-semantic-tr" if class_name is None else f"ui-semantic-tr {class_name}"), **props)


@component
def ColumnHeader(content: Any, *, scope: str = "col", class_name: str | None = None, **props: Any) -> object:
    return Th(content, scope=scope, class_name=("ui-col-header" if class_name is None else f"ui-col-header {class_name}"), **props)


@component
def RowHeader(content: Any, *, scope: str = "row", class_name: str | None = None, **props: Any) -> object:
    return Th(content, scope=scope, class_name=("ui-row-header" if class_name is None else f"ui-row-header {class_name}"), **props)


@component
def DataCell(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    return Td(content, class_name=("ui-data-cell" if class_name is None else f"ui-data-cell {class_name}"), **props)


@component
def ScreenReaderText(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    # Visually hidden, still available to assistive tech.
    _apply_class(props, "ui-sr-text", class_name)
    props.setdefault(
        "style",
        "position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0;",
    )
    return el("span", content, **props)


SrText = ScreenReaderText


def Decorative(node: Any, *, role: str | None = None, **props: Any) -> object:
    if isinstance(node, Element):
        merged = dict(node.props)
        merged.update(props)
        merged["aria_hidden"] = "true"
        if _normalize_tag(node.tag) in {"img", "svg"}:
            merged.setdefault("role", role or "presentation")
            if _normalize_tag(node.tag) == "img":
                merged.setdefault("alt", "")
        merged.setdefault("data_fb_a11y_decorative", "true")
        return Element(node.tag, merged, list(node.children))
    wrapped_props = dict(props)
    wrapped_props["aria_hidden"] = "true"
    wrapped_props.setdefault("data_fb_a11y_decorative", "true")
    if role:
        wrapped_props.setdefault("role", role)
    return el("span", node, **wrapped_props)


@component
def Main(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-main", class_name)
    return el("main", list(children), **props)


@component
def Nav(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-nav", class_name)
    return el("nav", list(children), **props)


@component
def Aside(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-aside", class_name)
    return el("aside", list(children), **props)


@component
def Region(
    *children: Any,
    label: str | None = None,
    labelledby: str | A11yId | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    _apply_class(props, "ui-region", class_name)
    props.setdefault("role", "region")
    if label is not None:
        props.setdefault("aria_label", label)
    if labelledby is not None:
        props.setdefault("aria_labelledby", str(labelledby))
    return el("section", list(children), **props)


@component
def Heading(
    content: Any,
    *,
    level: int = 2,
    class_name: str | None = None,
    **props: Any,
) -> object:
    if level < 1 or level > 6:
        raise ValueError("Heading level must be between 1 and 6")
    _apply_class(props, "ui-heading", class_name)
    return el(f"h{level}", content, **props)


@component
def Section(
    *children: Any,
    heading: Any = None,
    heading_level: int = 2,
    class_name: str | None = None,
    **props: Any,
) -> object:
    _apply_class(props, "ui-section", class_name)
    nodes: list[Any] = []
    if heading is not None:
        nodes.append(Heading(heading, level=heading_level))
    nodes.extend(children)
    return el("section", nodes, **props)


@component
def FieldSet(*children: Any, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-fieldset", class_name)
    return el("fieldset", list(children), **props)


@component
def Legend(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-legend", class_name)
    return el("legend", content, **props)


@component
def Label(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-label", class_name)
    return el("label", content, **props)


@component
def HelpText(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-help-text", class_name)
    props.setdefault("data_fb_a11y_kind", "help-text")
    return el("p", content, **props)


@component
def ErrorText(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-error-text", class_name)
    props.setdefault("data_fb_a11y_kind", "error-text")
    return el("p", content, **props)


@component
def Details(*children: Any, open: bool | None = None, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-details", class_name)
    if open is not None:
        props["open"] = bool(open)
    return el("details", list(children), **props)


@component
def Summary(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-summary", class_name)
    return el("summary", content, **props)


@component
def Status(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-status", class_name)
    props.setdefault("role", "status")
    props.setdefault("aria_live", "polite")
    return el("div", content, **props)


@component
def Alert(content: Any, *, class_name: str | None = None, **props: Any) -> object:
    _apply_class(props, "ui-alert", class_name)
    props.setdefault("role", "alert")
    props.setdefault("aria_live", "assertive")
    return el("div", content, **props)


@component
def LiveRegion(
    content: Any,
    *,
    live: str = "polite",
    role: str | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    _apply_class(props, "ui-live-region", class_name)
    if role:
        props.setdefault("role", role)
    props.setdefault("aria_live", live)
    return el("div", content, **props)


@component
def SignatureStatus(
    *,
    signature_status: str,
    signer_name: str | None = None,
    timestamp: str | None = None,
    signature_method: str | None = None,
    reference_id: str | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    if signature_status not in SIGNATURE_STATUS_VALUES:
        raise ValueError(f"Invalid signature_status {signature_status!r}")
    if signature_method is not None and signature_method not in SIGNATURE_METHOD_VALUES:
        raise ValueError(f"Invalid signature_method {signature_method!r}")
    _apply_class(props, "ui-signature-status", class_name)
    props.setdefault("data_fb_a11y_signature_status", signature_status)
    if signature_method is not None:
        props.setdefault("data_fb_a11y_signature_method", signature_method)
    items: list[Any] = [
        FieldItem("Signature status", signature_status.replace("_", " ")),
    ]
    if signer_name is not None:
        items.append(FieldItem("Signer", signer_name))
    if timestamp is not None:
        items.append(FieldItem("Timestamp", timestamp))
    if signature_method is not None:
        items.append(FieldItem("Method", signature_method.replace("_", " ")))
    if reference_id is not None:
        items.append(FieldItem("Reference ID", reference_id))
    return Section(
        Heading("Signature", level=3),
        FieldGrid(*items),
        **props,
    )


@component
def SignatureMark(
    *,
    src: str | None = None,
    node: Element | None = None,
    signer_name: str | None = None,
    decorative: bool = False,
    alt: str | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    if (src is None) == (node is None):
        raise ValueError("SignatureMark requires exactly one of src= or node=")
    if src is not None:
        _apply_class(props, "ui-signature-mark", class_name)
        props.setdefault("src", src)
        if decorative:
            props.setdefault("aria_hidden", "true")
            props.setdefault("role", "presentation")
            props.setdefault("alt", "")
            props.setdefault("data_fb_a11y_decorative", "true")
        else:
            computed_alt = alt or (f"Signature: {signer_name}" if signer_name else "Signature")
            props.setdefault("alt", computed_alt)
        return el("img", **props)
    assert node is not None
    marked: Any = node
    if decorative:
        marked = Decorative(node)
    else:
        computed_alt = alt or (f"Signature: {signer_name}" if signer_name else None)
        if computed_alt and _normalize_tag(node.tag) in {"img", "svg"}:
            if _normalize_tag(node.tag) == "img":
                marked = _clone_element(node, alt=computed_alt)
            else:
                marked = _clone_element(node, aria_label=computed_alt)
    if class_name and isinstance(marked, Element):
        merged_props = dict(marked.props)
        _apply_class(merged_props, "ui-signature-mark", class_name)
        marked = Element(marked.tag, merged_props, list(marked.children))
    return marked


@component
def SignatureBlock(
    *,
    signature_status: str,
    signer_name: str | None = None,
    timestamp: str | None = None,
    signature_method: str | None = None,
    reference_id: str | None = None,
    mark_src: str | None = None,
    mark_node: Element | None = None,
    mark_alt: str | None = None,
    mark_decorative: bool | None = None,
    class_name: str | None = None,
    **props: Any,
) -> object:
    _apply_class(props, "ui-signature-block", class_name)
    children: list[Any] = [
        SignatureStatus(
            signature_status=signature_status,
            signer_name=signer_name,
            timestamp=timestamp,
            signature_method=signature_method,
            reference_id=reference_id,
        )
    ]
    if mark_src is not None or mark_node is not None:
        decorative = True if mark_decorative is None else bool(mark_decorative)
        children.append(
            SignatureMark(
                src=mark_src,
                node=mark_node,
                signer_name=signer_name,
                decorative=decorative,
                alt=mark_alt,
            )
        )
    return Section(*children, **props)


__all__ = [
    "A11yAttrs",
    "A11yContract",
    "A11yId",
    "A11yValidationError",
    "A11yWarning",
    "Alert",
    "Aside",
    "ColumnHeader",
    "DataCell",
    "Decorative",
    "Dd",
    "DefinitionDescription",
    "DefinitionList",
    "DefinitionTerm",
    "Details",
    "Dl",
    "Dt",
    "ErrorText",
    "FieldGrid",
    "FieldItem",
    "FieldSet",
    "FigCaption",
    "Figure",
    "Heading",
    "HelpText",
    "Label",
    "Legend",
    "LiveRegion",
    "Main",
    "Nav",
    "Region",
    "RowHeader",
    "ScreenReaderText",
    "Section",
    "SemanticTable",
    "SemanticTableBody",
    "SemanticTableFoot",
    "SemanticTableHead",
    "SemanticTableRow",
    "SignatureBlock",
    "SignatureMark",
    "SignatureStatus",
    "SIGNATURE_METHOD_VALUES",
    "SIGNATURE_STATUS_VALUES",
    "SrText",
    "Status",
    "Summary",
    "TableCaption",
    "validate_a11y",
]
