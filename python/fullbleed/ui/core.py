from __future__ import annotations

import json
from dataclasses import dataclass, field
from html import escape
from pathlib import Path
from typing import Any, Callable

from .style import style_to_css


def _normalize_css_href(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_css_media(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _stylesheet_link_tag(css_href: str, css_media: str | None) -> str:
    media_attr = ""
    media_value = _normalize_css_media(css_media)
    if media_value:
        media_attr = f' media="{escape(media_value, quote=True)}"'
    return f'<link rel="stylesheet" href="{escape(css_href, quote=True)}"{media_attr} />'


def _inject_css_link(
    html_text: str,
    css_href: str | None,
    css_media: str | None,
) -> tuple[str, bool, bool]:
    href = _normalize_css_href(css_href)
    if not href:
        return html_text, False, False
    if 'rel="stylesheet"' in html_text or "rel='stylesheet'" in html_text:
        return html_text, False, True
    link = _stylesheet_link_tag(href, css_media)
    if "</head>" in html_text:
        return html_text.replace("</head>", f"{link}</head>", 1), True, False
    return html_text, False, False


@dataclass
class Element:
    tag: str
    props: dict[str, Any] = field(default_factory=dict)
    children: list[Any] = field(default_factory=list)

    def to_html(self, *, a11y_mode: str | None = None) -> str:
        return to_html(self, a11y_mode=a11y_mode)


@dataclass
class DocumentArtifact:
    root: Element
    page: str
    margin: str
    title: str
    bootstrap: bool
    lang: str = "en"
    css_href: str | None = None
    css_source_path: str | None = None
    css_media: str | None = "all"
    css_required: bool = False

    def to_html(self, *, a11y_mode: str | None = None) -> str:
        return compile_document(self, a11y_mode=a11y_mode)

    def document_metadata(self) -> dict[str, Any]:
        return {
            "document_lang": (self.lang or "en").strip() or "en",
            "document_title": self.title,
            "document_css_href": _normalize_css_href(self.css_href),
            "document_css_source_path": _normalize_css_href(self.css_source_path),
            "document_css_media": _normalize_css_media(self.css_media),
            "document_css_required": bool(self.css_required),
        }

    def _resolve_css_href(
        self,
        *,
        css_href_override: str | None = None,
        css_path_hint: str | Path | None = None,
    ) -> str | None:
        explicit = _normalize_css_href(css_href_override)
        if explicit:
            return explicit
        from_meta = _normalize_css_href(self.css_href)
        if from_meta:
            return from_meta
        if css_path_hint is None:
            return None
        basename = Path(css_path_hint).name.strip()
        return basename or None

    def _resolve_css_media(self, *, css_media_override: str | None = None) -> str | None:
        explicit = _normalize_css_media(css_media_override)
        if explicit:
            return explicit
        return _normalize_css_media(self.css_media)

    def _ensure_css_requirements(self, css_href: str | None) -> None:
        if bool(self.css_required) and not _normalize_css_href(css_href):
            raise ValueError(
                "DocumentArtifact css_required=True requires document_css_href "
                "or an explicit css_href override."
            )

    def emit_html(
        self,
        path: str | Path,
        *,
        a11y_mode: str | None = None,
        css_href: str | None = None,
        css_media: str | None = None,
        encoding: str = "utf-8",
    ) -> str:
        effective_css_href = self._resolve_css_href(css_href_override=css_href)
        self._ensure_css_requirements(effective_css_href)
        html = self.to_html(a11y_mode=a11y_mode)
        html, _, _ = _inject_css_link(
            html,
            effective_css_href,
            self._resolve_css_media(css_media_override=css_media),
        )
        out_path = Path(path)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(html, encoding=encoding)
        return html

    def emit_css(
        self,
        css: str | None,
        path: str | Path | None = None,
        *,
        encoding: str = "utf-8",
    ) -> str:
        out_path_raw = path if path is not None else self.css_source_path
        if out_path_raw is None:
            raise ValueError("emit_css requires a target path or document_css_source_path metadata.")
        out_path = Path(out_path_raw)

        if css is None:
            source_path = _normalize_css_href(self.css_source_path)
            if not source_path:
                raise ValueError(
                    "emit_css(css=None) requires document_css_source_path metadata to read source CSS."
                )
            css_text = Path(source_path).read_text(encoding=encoding)
        else:
            css_text = str(css)

        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(css_text, encoding=encoding)
        return css_text

    def emit_artifacts(
        self,
        *,
        css: str | None = None,
        html_path: str | Path,
        css_path: str | Path | None = None,
        css_href: str | None = None,
        css_media: str | None = None,
        a11y_mode: str | None = None,
        encoding: str = "utf-8",
    ) -> dict[str, Any]:
        css_out_path_raw = css_path if css_path is not None else self.css_source_path
        if css_out_path_raw is None:
            raise ValueError(
                "emit_artifacts requires css_path or document_css_source_path metadata."
            )
        css_out_path = Path(css_out_path_raw)
        effective_css_href = self._resolve_css_href(
            css_href_override=css_href,
            css_path_hint=css_out_path,
        )
        self._ensure_css_requirements(effective_css_href)

        html = self.emit_html(
            html_path,
            a11y_mode=a11y_mode,
            css_href=effective_css_href,
            css_media=css_media,
            encoding=encoding,
        )
        css_text = self.emit_css(css, css_path, encoding=encoding)
        return {
            "html_path": str(Path(html_path)),
            "css_path": str(css_out_path),
            "html": html,
            "css": css_text,
            "document_css_href": effective_css_href,
            "document_css_media": self._resolve_css_media(css_media_override=css_media),
        }


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
            rendered = _render_attr_value(attr, value)
            if rendered is None:
                continue
            parts.append(f'{attr}="{escape(rendered, quote=True)}"')
    return (" " + " ".join(parts)) if parts else ""


def _render_attr_value(attr: str, value: Any) -> str | None:
    if attr == "style":
        rendered = style_to_css(value)
        return rendered or None
    return str(value)


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
    lang: str = "en",
    css_href: str | None = None,
    css_source_path: str | None = None,
    css_media: str | None = "all",
    css_required: bool = False,
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
                lang=lang,
                css_href=css_href,
                css_source_path=css_source_path,
                css_media=css_media,
                css_required=css_required,
            )

        wrapped.__name__ = fn.__name__
        return wrapped

    return decorator


def _validate_a11y_if_requested(node_or_document: Any, a11y_mode: str | None) -> None:
    if a11y_mode is None:
        return
    mode = str(a11y_mode).strip().lower()
    if mode in {"", "none"}:
        return
    if mode not in {"warn", "raise"}:
        raise ValueError(f"Unsupported a11y_mode {a11y_mode!r}. Expected None, 'warn', or 'raise'.")
    try:
        from .accessibility import A11yContract
    except Exception:
        # If the accessibility layer isn't available yet, surface a clear error.
        raise RuntimeError("Accessibility validation requested but fullbleed.ui.accessibility is unavailable")
    A11yContract().validate(node_or_document, mode=mode)


def compile_document(artifact: DocumentArtifact, *, a11y_mode: str | None = None) -> str:
    _validate_a11y_if_requested(artifact, a11y_mode)
    lang = (artifact.lang or "en").strip() or "en"
    css_link = ""
    href = _normalize_css_href(artifact.css_href)
    if href:
        css_link = _stylesheet_link_tag(href, artifact.css_media)
    return (
        "<!doctype html>"
        f"<html lang=\"{escape(lang, quote=True)}\">"
        "<head>"
        "<meta charset=\"utf-8\" />"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />"
        f"<title>{escape(artifact.title)}</title>"
        f"{css_link}"
        "</head>"
        "<body>"
        f"{render_node(artifact.root)}"
        "</body>"
        "</html>"
    )


def to_html(node_or_document: Any, *, a11y_mode: str | None = None) -> str:
    if isinstance(node_or_document, DocumentArtifact):
        return compile_document(node_or_document, a11y_mode=a11y_mode)
    _validate_a11y_if_requested(node_or_document, a11y_mode)
    if isinstance(node_or_document, Element):
        return render_node(node_or_document)
    return render_node(node_or_document)


def _mount_to_artifact(
    node_or_component: Any,
    *,
    props: Any = None,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "component mount harness",
    lang: str = "en",
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
        lang=lang,
    )


def mount_component_html(
    node_or_component: Any,
    *,
    props: Any = None,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "component mount harness",
    lang: str = "en",
    a11y_mode: str | None = None,
) -> str:
    artifact = _mount_to_artifact(
        node_or_component,
        props=props,
        page=page,
        margin=margin,
        title=title,
        lang=lang,
    )
    return compile_document(artifact, a11y_mode=a11y_mode)


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


def _trace_block_bbox(block: dict[str, Any]) -> tuple[float, float, float, float] | None:
    bbox = block.get("bbox")
    if isinstance(bbox, dict):
        x = _as_float(bbox.get("x"))
        y = _as_float(bbox.get("y"))
        w = _as_float(bbox.get("w"))
        h = _as_float(bbox.get("h"))
    else:
        x = _as_float(block.get("x"))
        y = _as_float(block.get("y"))
        w = _as_float(block.get("w"))
        h = _as_float(block.get("h"))
    if x is None or y is None or w is None or h is None:
        return None
    if w <= 0.0 or h <= 0.0:
        return None
    return (x, y, x + w, y + h)


def _collect_render_trace_overlap_signals(render_trace: Any) -> dict[str, Any]:
    text_overlap_count = 0
    text_overlap_samples: list[dict[str, Any]] = []
    trace_pages_scanned = 0
    trace_blocks_scanned = 0
    trace_bbox_missing_count = 0

    if not isinstance(render_trace, dict):
        return {
            "render_time_trace_available": False,
            "trace_pages_scanned": 0,
            "trace_blocks_scanned": 0,
            "trace_bbox_missing_count": 0,
            "text_overlap_count": 0,
            "text_overlap_samples": [],
        }

    for page in render_trace.get("pages", []) or []:
        if not isinstance(page, dict):
            continue
        trace_pages_scanned += 1
        blocks = page.get("blocks", []) or []
        page_num = _as_int(page.get("page")) or (_as_int(page.get("page_index")) or 0) + 1

        indexed: list[dict[str, Any]] = []
        for block in blocks:
            if not isinstance(block, dict):
                continue
            kind = str(block.get("kind", ""))
            if kind not in {"draw_string", "draw_string_transformed"}:
                continue
            text = str(block.get("text", ""))
            if not text or not text.strip():
                continue
            bbox = _trace_block_bbox(block)
            if bbox is None:
                trace_bbox_missing_count += 1
                continue
            indexed.append(
                {
                    "page": page_num,
                    "index": _as_int(block.get("index")) or 0,
                    "command_index": _as_int(block.get("command_index")) or 0,
                    "text": text,
                    "top_role": block.get("top_role"),
                    "bbox": bbox,
                }
            )
        trace_blocks_scanned += len(indexed)

        for i in range(len(indexed)):
            a = indexed[i]
            ax0, ay0, ax1, ay1 = a["bbox"]
            for j in range(i + 1, len(indexed)):
                b = indexed[j]
                bx0, by0, bx1, by1 = b["bbox"]
                ix0 = max(ax0, bx0)
                iy0 = max(ay0, by0)
                ix1 = min(ax1, bx1)
                iy1 = min(ay1, by1)
                iw = ix1 - ix0
                ih = iy1 - iy0
                if iw <= 0.0 or ih <= 0.0:
                    continue
                # Ignore edge-touch and tiny float jitter; keep real visual collisions.
                if iw < 1.0 or ih < 1.0 or (iw * ih) < 4.0:
                    continue
                text_overlap_count += 1
                if len(text_overlap_samples) < 8:
                    text_overlap_samples.append(
                        {
                            "page": page_num,
                            "overlap_bbox": {"x": ix0, "y": iy0, "w": iw, "h": ih},
                            "a": {
                                "index": a["index"],
                                "command_index": a["command_index"],
                                "top_role": a.get("top_role"),
                                "text": a["text"][:80],
                            },
                            "b": {
                                "index": b["index"],
                                "command_index": b["command_index"],
                                "top_role": b.get("top_role"),
                                "text": b["text"][:80],
                            },
                        }
                    )

    return {
        "render_time_trace_available": True,
        "trace_pages_scanned": trace_pages_scanned,
        "trace_blocks_scanned": trace_blocks_scanned,
        "trace_bbox_missing_count": trace_bbox_missing_count,
        "text_overlap_count": text_overlap_count,
        "text_overlap_samples": text_overlap_samples,
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
    fail_on_text_overlap: bool = True,
) -> dict[str, Any]:
    html = mount_component_html(
        node_or_component,
        props=props,
        page=page,
        margin=margin,
        title=title,
    )
    render_time_trace = None
    render_time_trace_error: str | None = None
    combined_render = getattr(
        engine,
        "render_pdf_with_glyph_report_and_render_time_reading_order_trace",
        None,
    )
    if callable(combined_render):
        pdf_bytes, glyph_report, render_time_trace = combined_render(html, css)
    else:
        pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report(html, css)
        export_trace = getattr(engine, "export_render_time_reading_order_trace", None)
        if callable(export_trace):
            try:
                render_time_trace = export_trace(html, css)
            except Exception as exc:  # pragma: no cover - defensive native/runtime path
                render_time_trace_error = f"{type(exc).__name__}: {exc}"
    glyph_list = list(glyph_report or [])

    debug_entries = _read_jsonl(debug_log) if debug_log else []
    signals = _collect_debug_signals(debug_entries)
    trace_signals = _collect_render_trace_overlap_signals(render_time_trace)
    overflow_count = int(signals["overflow_count"])
    overflow_samples = list(signals["overflow_samples"])
    css_warning_count = int(signals["css_warning_count"])
    css_warning_samples = list(signals["css_warning_samples"])
    known_loss_count = int(signals["known_loss_count"])
    known_loss_samples = list(signals["known_loss_samples"])
    html_asset_warning_count = int(signals["html_asset_warning_count"])
    html_asset_warning_samples = list(signals["html_asset_warning_samples"])
    text_overlap_count = int(trace_signals["text_overlap_count"])
    text_overlap_samples = list(trace_signals["text_overlap_samples"])
    render_time_trace_available = bool(trace_signals["render_time_trace_available"])
    trace_pages_scanned = int(trace_signals["trace_pages_scanned"])
    trace_blocks_scanned = int(trace_signals["trace_blocks_scanned"])
    trace_bbox_missing_count = int(trace_signals["trace_bbox_missing_count"])

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
    if text_overlap_count > 0:
        signal = {
            "code": "TEXT_OVERLAP",
            "count": text_overlap_count,
            "samples": text_overlap_samples,
        }
        if fail_on_text_overlap:
            failures.append(signal)
        else:
            warnings.append(signal)
    if render_time_trace_error:
        warnings.append(
            {
                "code": "RENDER_TIME_TRACE_ERROR",
                "message": render_time_trace_error,
            }
        )

    return {
        "ok": not failures,
        "bytes_written": len(pdf_bytes),
        "missing_glyph_count": len(glyph_list),
        "overflow_count": overflow_count,
        "css_warning_count": css_warning_count,
        "css_miss_count": css_warning_count,
        "known_loss_count": known_loss_count,
        "html_asset_warning_count": html_asset_warning_count,
        "text_overlap_count": text_overlap_count,
        "render_time_trace_available": render_time_trace_available,
        "render_time_trace_pages_scanned": trace_pages_scanned,
        "render_time_trace_blocks_scanned": trace_blocks_scanned,
        "render_time_trace_bbox_missing_count": trace_bbox_missing_count,
        "render_time_trace_error": render_time_trace_error,
        "debug_log": debug_log,
        "failures": failures,
        "warnings": warnings,
    }
