from __future__ import annotations

import json
from dataclasses import dataclass, field
from html import escape
from pathlib import Path
from typing import Any, Callable


@dataclass
class Element:
    tag: str
    props: dict[str, Any] = field(default_factory=dict)
    children: list[Any] = field(default_factory=list)


@dataclass
class DocumentArtifact:
    root: Element
    page: str
    margin: str
    title: str
    bootstrap: bool


def component(fn: Callable) -> Callable:
    """Marker decorator for function components."""
    fn.__fullbleed_component__ = True
    return fn


def el(tag: str, *children: Any, **props: Any) -> Element:
    flat: list[Any] = []
    for child in children:
        if child is None:
            continue
        if isinstance(child, (list, tuple)):
            flat.extend(x for x in child if x is not None)
        else:
            flat.append(child)
    return Element(tag=tag, props=props, children=flat)


def _normalize_attr_name(name: str) -> str:
    if name == "class_name":
        return "class"
    if name.startswith("data_fb_"):
        return "data-fb-" + name[len("data_fb_") :].replace("_", "-")
    return name.replace("_", "-")


def _render_attrs(props: dict[str, Any]) -> str:
    parts: list[str] = []
    for key, value in props.items():
        if value is None or value is False:
            continue
        attr = _normalize_attr_name(key)
        if value is True:
            parts.append(attr)
        else:
            parts.append(f'{attr}="{escape(str(value), quote=True)}"')
    return (" " + " ".join(parts)) if parts else ""


def render_node(node: Any) -> str:
    if node is None:
        return ""
    if isinstance(node, Element):
        attrs = _render_attrs(node.props)
        children_html = "".join(render_node(child) for child in node.children)
        return f"<{node.tag}{attrs}>{children_html}</{node.tag}>"
    return escape(str(node))


def Document(
    *,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "fullbleed document",
    bootstrap: bool = True,
) -> Callable[[Callable[..., Any]], Callable[..., DocumentArtifact]]:
    """Wrap an app component into a document artifact.

    Note:
    - `page`/`margin` here are document metadata and authoring hints.
    - Actual engine page geometry is configured in `create_engine()` in report.py.
    """
    def decorator(fn: Callable[..., Any]) -> Callable[..., DocumentArtifact]:
        def wrapped(*args: Any, **kwargs: Any) -> DocumentArtifact:
            tree = fn(*args, **kwargs)
            root_class = "fb-document-root report-root"
            if bootstrap:
                root_class = f"{root_class} fb-bootstrap-enabled"
            root = el(
                "main",
                tree,
                class_name=root_class,
                data_fb_role="document-root",
                data_fb_page=page,
            )
            return DocumentArtifact(
                root=root,
                page=page,
                margin=margin,
                title=title,
                bootstrap=bootstrap,
            )

        wrapped.__name__ = fn.__name__
        return wrapped

    return decorator


def compile_document(artifact: DocumentArtifact) -> str:
    return (
        "<!doctype html>"
        "<html lang=\"en\">"
        "<head>"
        "<meta charset=\"utf-8\" />"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />"
        f"<title>{escape(artifact.title)}</title>"
        "</head>"
        "<body>"
        f"{render_node(artifact.root)}"
        "</body>"
        "</html>"
    )


def _mount_to_artifact(
    node_or_component: Any,
    *,
    props: Any = None,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "component mount harness",
) -> DocumentArtifact:
    mounted = node_or_component
    if callable(node_or_component):
        if props is None:
            mounted = node_or_component()
        else:
            mounted = node_or_component(props)
    if isinstance(mounted, DocumentArtifact):
        return mounted
    return DocumentArtifact(
        root=el(
            "main",
            mounted,
            class_name="fb-mount-root",
            data_fb_role="mount-root",
            data_fb_page=page,
        ),
        page=page,
        margin=margin,
        title=title,
        bootstrap=False,
    )


def mount_component_html(
    node_or_component: Any,
    *,
    props: Any = None,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "component mount harness",
) -> str:
    artifact = _mount_to_artifact(
        node_or_component,
        props=props,
        page=page,
        margin=margin,
        title=title,
    )
    return compile_document(artifact)


def _read_jsonl(path: str) -> list[dict[str, Any]]:
    p = Path(path)
    if not p.exists():
        return []
    rows: list[dict[str, Any]] = []
    for line in p.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict):
            rows.append(obj)
    return rows


def _as_float(value: Any) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _as_int(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _selector_is_authored(selector: str) -> bool:
    selector_l = selector.lower()
    return ".fb-" in selector_l or ".ui-" in selector_l or "[data-fb-role=" in selector_l


def _collect_debug_signals(debug_entries: list[dict[str, Any]]) -> dict[str, Any]:
    overflow_count = 0
    overflow_samples: list[dict[str, Any]] = []
    css_warning_count = 0
    css_warning_samples: list[dict[str, Any]] = []
    known_loss_count = 0
    known_loss_samples: list[dict[str, Any]] = []
    html_asset_warning_count = 0
    html_asset_warning_samples: list[dict[str, Any]] = []

    for entry in debug_entries:
        entry_type = str(entry.get("type", ""))
        entry_type_l = entry_type.lower()

        if entry_type == "debug.summary":
            counts = entry.get("counts", {})
            if isinstance(counts, dict):
                for key, value in counts.items():
                    key_l = str(key).lower()
                    if "css" not in key_l:
                        continue
                    if not any(
                        token in key_l
                        for token in ("miss", "missing", "unknown", "invalid", "unresolved")
                    ):
                        continue
                    ivalue = _as_int(value) or 0
                    if ivalue <= 0:
                        continue
                    css_warning_count += ivalue
                    if len(css_warning_samples) < 8:
                        css_warning_samples.append({"metric": str(key), "count": ivalue})

        if "css" in entry_type_l and any(
            token in entry_type_l for token in ("miss", "missing", "unknown", "invalid", "unresolved")
        ):
            css_warning_count += 1
            if len(css_warning_samples) < 8:
                css_warning_samples.append({"type": entry_type})

        if entry_type == "jit.known_loss":
            selector = str(entry.get("selector", ""))
            if _selector_is_authored(selector):
                known_loss_count += 1
                if len(known_loss_samples) < 8:
                    known_loss_samples.append(
                        {
                            "code": entry.get("code"),
                            "property": entry.get("property"),
                            "selector": selector,
                        }
                    )
            continue

        if entry_type == "jit.html_asset_warning":
            html_asset_warning_count += 1
            if len(html_asset_warning_samples) < 8:
                html_asset_warning_samples.append(
                    {
                        "kind": entry.get("kind"),
                        "message": entry.get("message"),
                    }
                )
            continue

        if entry_type != "jit.docplan":
            continue
        page_size = entry.get("page_size", {})
        if not isinstance(page_size, dict):
            continue
        page_w = _as_float(page_size.get("w"))
        page_h = _as_float(page_size.get("h"))
        if page_w is None or page_h is None:
            continue
        for page in entry.get("pages", []) or []:
            if not isinstance(page, dict):
                continue
            page_num = page.get("n")
            for placement in page.get("placements", []) or []:
                if not isinstance(placement, dict):
                    continue
                bbox = placement.get("bbox")
                if not isinstance(bbox, dict):
                    continue
                x = _as_float(bbox.get("x"))
                y = _as_float(bbox.get("y"))
                w = _as_float(bbox.get("w"))
                h = _as_float(bbox.get("h"))
                if x is None or y is None or w is None or h is None:
                    continue
                if x < -0.01 or y < -0.01 or (x + w) > (page_w + 0.01) or (y + h) > (page_h + 0.01):
                    overflow_count += 1
                    if len(overflow_samples) < 5:
                        overflow_samples.append(
                            {
                                "page": page_num,
                                "bbox": {"x": x, "y": y, "w": w, "h": h},
                                "page_size": {"w": page_w, "h": page_h},
                            }
                        )

    return {
        "overflow_count": overflow_count,
        "overflow_samples": overflow_samples,
        "css_warning_count": css_warning_count,
        "css_warning_samples": css_warning_samples,
        "known_loss_count": known_loss_count,
        "known_loss_samples": known_loss_samples,
        "html_asset_warning_count": html_asset_warning_count,
        "html_asset_warning_samples": html_asset_warning_samples,
    }


def validate_component_mount(
    *,
    engine: Any,
    node_or_component: Any,
    css: str = "",
    props: Any = None,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "component mount harness",
    debug_log: str | None = None,
    fail_on_overflow: bool = False,
    fail_on_css_warnings: bool = False,
    fail_on_known_loss: bool = False,
    fail_on_html_asset_warning: bool = True,
) -> dict[str, Any]:
    html = mount_component_html(
        node_or_component,
        props=props,
        page=page,
        margin=margin,
        title=title,
    )
    pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report(html, css)
    glyph_list = list(glyph_report or [])

    debug_entries = _read_jsonl(debug_log) if debug_log else []
    signals = _collect_debug_signals(debug_entries)
    overflow_count = int(signals["overflow_count"])
    overflow_samples = list(signals["overflow_samples"])
    css_warning_count = int(signals["css_warning_count"])
    css_warning_samples = list(signals["css_warning_samples"])
    known_loss_count = int(signals["known_loss_count"])
    known_loss_samples = list(signals["known_loss_samples"])
    html_asset_warning_count = int(signals["html_asset_warning_count"])
    html_asset_warning_samples = list(signals["html_asset_warning_samples"])

    failures: list[dict[str, Any]] = []
    warnings: list[dict[str, Any]] = []
    if glyph_list:
        failures.append(
            {
                "code": "MISSING_GLYPHS",
                "count": len(glyph_list),
                "sample": glyph_list[:10],
            }
        )
    if overflow_count > 0:
        signal = {
            "code": "OVERFLOW",
            "count": overflow_count,
            "samples": overflow_samples,
        }
        if fail_on_overflow:
            failures.append(signal)
        else:
            warnings.append(signal)
    if css_warning_count > 0:
        signal = {
            "code": "CSS_WARNING",
            "count": css_warning_count,
            "samples": css_warning_samples,
        }
        if fail_on_css_warnings:
            failures.append(signal)
        else:
            warnings.append(signal)
    if known_loss_count > 0:
        signal = {
            "code": "ENGINE_KNOWN_LOSS",
            "count": known_loss_count,
            "samples": known_loss_samples,
        }
        if fail_on_known_loss:
            failures.append(signal)
        else:
            warnings.append(signal)
    if html_asset_warning_count > 0:
        signal = {
            "code": "HTML_ASSET_WARNING",
            "count": html_asset_warning_count,
            "samples": html_asset_warning_samples,
        }
        if fail_on_html_asset_warning:
            failures.append(signal)
        else:
            warnings.append(signal)

    return {
        "ok": not failures,
        "bytes_written": len(pdf_bytes),
        "missing_glyph_count": len(glyph_list),
        "overflow_count": overflow_count,
        "css_warning_count": css_warning_count,
        "css_miss_count": css_warning_count,
        "known_loss_count": known_loss_count,
        "html_asset_warning_count": html_asset_warning_count,
        "debug_log": debug_log,
        "failures": failures,
        "warnings": warnings,
    }
