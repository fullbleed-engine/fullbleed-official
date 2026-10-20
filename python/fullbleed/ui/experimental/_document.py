# SPDX-License-Identifier: MIT
from __future__ import annotations

from dataclasses import dataclass, field
from hashlib import sha256
from html import escape
import json
import warnings
from pathlib import Path
from types import MappingProxyType
from copy import deepcopy
from collections.abc import Mapping

from ..core import Element, _render_attrs, _HTML_VOID_ELEMENTS
from ..stylesheet import StyleSheet
from ..style import StyleWarning
from ._style import AuthoringError, Length, Style, Theme, mm
from ._tree import Node, _children, _required_tokens
from ._semantics import ContentContext, normalize_mode, semantic_report
from ._resources import style_element
from ._page import PageBand, PageNumbers, band_nodes, slot_css, HTML_BAND_CSS


def _capture_document_styles(value) -> tuple:
    """Snapshot the existing selector stylesheet API, preserving native CSS rules."""
    if value is None:
        return ()
    try:
        if isinstance(value, Mapping):
            fragments = []
            for selector, declarations in value.items():
                if not isinstance(selector, str) or not selector.strip():
                    raise TypeError("Stylesheet selectors must be nonempty strings")
                if isinstance(declarations, Style):
                    fragments.append((selector.strip(), Style(declarations)))
                else:
                    with warnings.catch_warnings():
                        warnings.simplefilter("error", StyleWarning)
                        fragments.append(StyleSheet({selector: declarations}).to_css())
            return tuple(fragments)
        if isinstance(value, str):
            text = value.strip()
        elif callable(getattr(value, "to_css", None)):
            text = value.to_css()
        else:
            raise TypeError("Document styles expects CSS text, a selector mapping, or a stylesheet with to_css()")
        if not isinstance(text, str):
            raise TypeError("Stylesheet to_css() must return CSS text")
    except (TypeError, ValueError, StyleWarning) as error:
        raise AuthoringError(str(error), code="styles.type", path=("Document", "styles")) from error
    return (text,)


def _document_styles(fragments, theme) -> str:
    resolved = []
    for fragment in fragments:
        if isinstance(fragment, str):
            resolved.append(fragment)
        else:
            selector, declarations = fragment
            try:
                resolved.append(f"{selector} {{ {declarations.resolve(theme)} }}")
            except AuthoringError as error:
                raise error.at(selector).at("styles").at("Document") from error
    text = "\n".join(resolved)
    if text.strip():
        from ..._abi import _stylesheet_page_geometry
        try:
            conflicts = _stylesheet_page_geometry(text)
        except ValueError as error:
            raise AuthoringError(str(error), code="styles.syntax", path=("Document", "styles")) from error
        if conflicts:
            selector, property_name, offset = conflicts[0]
            rule = "@page" + (" " + selector if selector else "")
            raise AuthoringError(
                f"Page owns paper size, orientation and margins; remove {property_name!r} from "
                f"{rule} at CSS byte {offset} and configure Document(page=Page(...)). "
                "Page furniture and ordinary element margins remain available in styles.",
                code="styles.page", path=("Document", "styles", rule, property_name),
            )
    return text


@dataclass(frozen=True)
class Page:
    """Page geometry, compiled to an actual @page rule."""

    size: str | tuple[Length, Length] = "A4"
    margin: Length | tuple[Length, ...] = mm(18)
    landscape: bool = False
    header: PageBand | None = field(default=None, kw_only=True)
    footer: PageBand | None = field(default=None, kw_only=True)

    def __post_init__(self):
        if isinstance(self.size, str):
            if self.size.upper() not in {"A3", "A4", "A5", "LETTER", "LEGAL", "TABLOID"}:
                raise AuthoringError("Use a supported paper name or a (width, height) length pair", code="page.size")
            object.__setattr__(self, "size", self.size.upper())
        elif not isinstance(self.size, tuple) or len(self.size) != 2:
            raise AuthoringError("Custom page size requires two physical lengths", code="page.size")
        else:
            for length in self.size:
                self._physical(length, positive=True)
        margins = self.margin if isinstance(self.margin, tuple) else (self.margin,)
        if not 1 <= len(margins) <= 4:
            raise AuthoringError("Page margins require one to four physical lengths", code="page.margin")
        for length in margins:
            self._physical(length)
        if not isinstance(self.landscape, bool):
            raise AuthoringError("landscape must be a boolean", code="page.orientation")
        for name, value in (("header", self.header), ("footer", self.footer)):
            if value is not None and not isinstance(value, PageBand):
                raise AuthoringError(f"Page {name} expects a PageBand value", code="page.band")

    @staticmethod
    def _physical(length, positive=False):
        if not isinstance(length, Length) or length.unit not in {"pt", "mm", "cm", "in", "px"}:
            raise AuthoringError("Page geometry requires physical lengths such as mm(18)", code="page.unit")
        if length.value < 0 or positive and length.value == 0:
            raise AuthoringError("Page sizes must be positive and margins nonnegative", code="page.range")

    def to_css(self) -> str:
        size = self.size
        if isinstance(size, tuple):
            size = " ".join(map(str, reversed(size) if self.landscape else size))
        elif self.landscape:
            size += " landscape"
        margins = self.margin if isinstance(self.margin, tuple) else (self.margin,)
        return f"@page {{ size: {size}; margin: {' '.join(map(str, margins))}; }}"


@dataclass(frozen=True)
class Source:
    node_id: str
    path: tuple[str, ...]
    file: str | None
    line: int | None


class LayoutError(AuthoringError):
    """A native pagination failure with fragment scopes and split rejection.

    sources retains the fragment's authored roots. failure_sources identifies
    constraints reported by the splitter, when available; it is not a heuristic
    search through descendants. File/line identifies component definitions.
    """

    def __init__(self, message, *, page, sources=(), reason=None, failure_sources=(),
                 code="layout.unsplittable", path=()):
        self.page, self.sources = page, tuple(sources)
        self.reason, self.failure_sources = reason, tuple(failure_sources)
        super().__init__(message, code=code, path=path)

    def at(self, name: str) -> LayoutError:
        return LayoutError(self.message, page=self.page, sources=self.sources,
                           reason=self.reason, failure_sources=self.failure_sources,
                           code=self.code, path=(name, *self.path))

    def to_dict(self) -> dict:
        return {
            **super().to_dict(), "page": self.page,
            "sources": [{"node_id": source.node_id, "path": list(source.path),
                         "file": source.file, "line": source.line} for source in self.sources],
            "reason": self.reason,
            "failure_sources": [{"node_id": source.node_id, "path": list(source.path),
                                 "file": source.file, "line": source.line} for source in self.failure_sources],
        }


class PageBandError(LayoutError):
    """Native page-furniture fit failure with the authored slot and measurements."""

    def __init__(self, message, *, details, sources=(), path=()):
        self._details = deepcopy(details)
        self.slot = details["slot"]
        self.record = details.get("record")
        super().__init__(message, page=details["page"], sources=sources,
                         reason=details["reason"], code=details["code"], path=path)

    @property
    def details(self):
        return deepcopy(self._details)

    def at(self, name):
        return PageBandError(self.message, details=self._details, sources=self.sources, path=(name, *self.path))

    def to_dict(self):
        return {**super().to_dict(), "slot": self.slot, "record": self.record, "details": self.details}


@dataclass(frozen=True)
class DocumentSource:
    """Pure authoring output; use Document.prepare for an owned native layout."""

    html: str
    css: str
    sources: tuple[Source, ...]
    _a11y_json: str | None = field(default=None, repr=False)
    _fragment: str = field(default="", repr=False, kw_only=True)
    _component_css: str = field(default="", repr=False, kw_only=True)

    @property
    def a11y_report(self) -> dict | None:
        """Independent report values, or None when semantic checks were omitted."""
        return json.loads(self._a11y_json) if self._a11y_json is not None else None


@dataclass(frozen=True, init=False)
class Document:
    """A frozen document value. Compilation needs no renderer or global registry."""

    children: tuple
    title: str
    lang: str
    page: Page
    theme: Theme
    _styles: tuple

    def __init__(self, *children, title="Fullbleed document", lang="en", page=None, theme=None, styles=None):
        if not isinstance(title, str) or not title.strip() or not isinstance(lang, str) or not lang.strip():
            raise AuthoringError("Document title and language must be nonempty strings", code="document.metadata")
        if page is not None and not isinstance(page, Page) or theme is not None and not isinstance(theme, Theme):
            raise AuthoringError("Document expects Page and Theme values", code="document.options")
        for name, value in {"children": _children(children), "title": title, "lang": lang,
                            "page": page or Page(), "theme": theme or Theme(),
                            "_styles": _capture_document_styles(styles)}.items():
            object.__setattr__(self, name, value)

    @property
    def styles(self) -> str:
        """Resolved selector CSS; source inputs were captured at construction."""
        return _document_styles(self._styles, self.theme)

    def compile(self, *, a11y_mode: str | None = None, target: str = "html") -> DocumentSource:
        return self._compile(a11y_mode=a11y_mode, target=target)

    def _compile(self, *, a11y_mode=None, include_styles=True, fragment_only=False, target="html") -> DocumentSource:
        if not isinstance(target, str) or target not in {"html", "pdf"}:
            raise AuthoringError("Compilation target must be 'html' or 'pdf'", code="document.target")
        mode = normalize_mode(a11y_mode)
        selector_styles = self.styles if include_styles else ""
        rules, sources, output, resources = {}, [], [], {}
        groups, band_slots = ({}, {}) if fragment_only else band_nodes(self.page, self.theme, target)
        has_bands = bool(groups)
        roots = self.children
        if has_bands:
            body_root = Node("main", self.children)
            # Register both running bands before body pagination starts. A footer
            # after the body would first bind on its last page (and can turn a
            # terminal forced break into an unwanted extra page).
            ordered = (groups.get("header"), groups.get("footer"), body_root) if target == "pdf" else (
                groups.get("header"), body_root, groups.get("footer"))
            roots = tuple(node for node in ordered if node is not None)
        margin_rules = []
        semantic_root = Element("div" if has_bands else "main") if mode else None
        # Explicit traversal avoids Python recursion limits for nested documents.
        root_context = ContentContext(parent=None if fragment_only else "body" if has_bands else "main")
        pending = [(child, (), None, root_context, semantic_root) for child in reversed(roots)]
        while pending:
            node, path, source, context, semantic_parent = pending.pop()
            if isinstance(node, _Closing):
                output.append(node.html)
                continue
            if isinstance(node, str):
                context.text(node, path)
                output.append(escape(node))
                if semantic_parent is not None:
                    semantic_parent.children.append(node)
                continue
            band_slot = band_slots.get(id(node))
            if band_slot is not None and target == "pdf":
                # Repeated PDF furniture is artifact content, outside body semantics.
                semantic_parent = None
            if target == "pdf":
                node = node._for_print()
            if node.name is not None:
                path = (*path, node.name)
                source = node.source or source
            for resource in node._resources if include_styles else ():
                key = (resource.kind, resource.name)
                if key in resources and resources[key] != resource:
                    raise AuthoringError(f"Conflicting component resource {resource.name!r}",
                                         code="resource.conflict", path=path)
                resources[key] = resource
            if node.tag is None:
                pending.extend((child, path, source, context, semantic_parent) for child in reversed(node.children))
                continue
            node_id = f"n{len(sources) + 1}"
            location = (*path, node.part or node.tag)
            if node.tag.casefold() == "style":
                raise AuthoringError("Attach selector CSS with Document(styles=...), rather than a style content node",
                                     code="structure.stylesheet", path=location)
            child_context = context.element(node.tag, location)
            sources.append(Source(node_id, location, source[0] if source else None, source[1] if source else None))
            attributes = dict(node.attributes)
            attributes["data-fb-id"] = node_id
            try:
                declarations = node._remaining_defaults(target=target).resolve(self.theme) if include_styles else ""
                explicit = node.style.resolve(self.theme) if include_styles else ""
            except AuthoringError as error:
                raise AuthoringError(error.message, code=error.code, path=location) from error
            if declarations:
                class_name = "fbx-" + sha256(declarations.encode()).hexdigest()[:24]
                if class_name in rules and rules[class_name] != declarations:
                    raise AuthoringError("Generated style identity collision", code="style.collision")
                rules[class_name] = declarations
                attributes["class"] = " ".join(filter(None, (attributes.get("class"), class_name)))
            if explicit:
                attributes["style"] = explicit
            if band_slot is not None and target == "pdf" and include_styles:
                edge, position, band, number = band_slot
                attributes["data-fb-running-layout"] = "band"
                attributes["style"] = explicit + f" position: running(fbx-{edge}-{position}) !important;"
                try:
                    margin_rules.extend(slot_css(edge, position, band, number, node_id))
                except AuthoringError as error:
                    raise error.at(f"Page.{edge}.{position}") from error
            output.append(f"<{node.tag}{_render_attrs(attributes)}>")
            if semantic_parent is not None:
                semantic_node = Element(node.tag, props=attributes)
                semantic_parent.children.append(semantic_node)
            else:
                semantic_node = None
            if node.tag.casefold() not in _HTML_VOID_ELEMENTS:
                pending.append((_Closing(f"</{node.tag}>"), path, source, context, semantic_parent))
                pending.extend((child, path, source, child_context, semantic_node) for child in reversed(node.children))
        body = Style(margin=0, box_sizing="border-box")
        text = Style(body, self.theme.text).resolve(self.theme) if include_styles and not fragment_only else ""
        dependency_css = tuple(resource.text for resource in resources.values() if resource.kind == "css")
        component_rules = tuple(f".{name} {{ {value} }}" for name, value in rules.items())
        component_css = "\n".join((*dependency_css, *component_rules)) if include_styles else ""
        if has_bands and include_styles:
            from ..._abi import _stylesheet_page_margins
            owned = {("top" if edge == "header" else "bottom") + "-" + position
                     for edge, position, _, _ in band_slots.values()}
            authored = "\n".join((component_css, selector_styles))
            try:
                conflicts = _stylesheet_page_margins(authored)
            except ValueError as error:
                raise AuthoringError(str(error), code="styles.syntax", path=("Document", "styles")) from error
            for selector, slot, offset in conflicts:
                if slot in owned:
                    raise AuthoringError(
                        f"PageBand owns @{slot}; remove the competing @page {selector} rule at CSS byte {offset} "
                        "and customize that band's component or PageNumbers value.",
                        code="styles.page_band", path=("Document", "styles", selector or "@page", slot),
                    )
        css = ""
        if include_styles:
            css = component_css if fragment_only else "\n".join((
                *dependency_css, self.page.to_css(), f"body {{ {text} }}",
                "* { box-sizing: border-box; }", *component_rules, selector_styles,
                *((HTML_BAND_CSS,) if has_bands and target == "html" else margin_rules),
            ))
        fragment = "".join(output)
        content = fragment if has_bands else "<main>" + fragment + "</main>"
        html = (f'<!doctype html><html lang="{escape(self.lang, quote=True)}"><head>'
                f'<meta charset="utf-8"><title>{escape(self.title)}</title></head><body>'
                + content + "</body></html>")
        report = semantic_report(semantic_root, self, sources, mode) if mode else None
        return DocumentSource(html, css, tuple(sources), json.dumps(report, sort_keys=True) if report is not None else None,
                              _fragment=fragment,
                              _component_css=component_css)

    @property
    def required_tokens(self) -> tuple[str, ...]:
        """Sorted explicit token names used by content and the theme's text style."""
        names = set(_required_tokens(self.children)) | set(self.theme.text._token_names())
        for band in (self.page.header, self.page.footer):
            if band is not None:
                names.update(band._tokens())
        for fragment in self._styles:
            if not isinstance(fragment, str):
                names.update(fragment[1]._token_names())
        return tuple(sorted(names))

    def to_html(self, *, include_styles: bool | None = None,
                a11y_mode: str | None = None) -> str:
        """Export HTML with embedded CSS, no styles, or the legacy separate-CSS form.

        True embeds collected CSS and retains inline declarations. False omits
        both, retaining authored class names. None preserves the existing output
        used with to_css(). Exports are static and do not add JavaScript.
        """
        _export_options(include_styles)
        source = self._compile(a11y_mode=a11y_mode, include_styles=include_styles is not False)
        html = source.html
        if include_styles is True:
            html = html.replace("</head>",
                                '<meta name="viewport" content="width=device-width, initial-scale=1">'
                                + style_element(source.css) + "</head>", 1)
        return html

    def to_css(self, *, a11y_mode: str | None = None) -> str:
        return self.compile(a11y_mode=a11y_mode).css

    def prepare(self, renderer: Renderer, *, a11y_mode: str | None = None):
        """Lay out and check once, returning the engine's owned CompiledDocument.

        Reuse the result for PDF bytes, layout previews and pagination diagnostics.
        A changed document or renderer needs a new preparation. No global cache
        or mutable engine is shared between documents.
        """
        plan = self.compile(a11y_mode=a11y_mode, target="pdf")
        try:
            prepared = renderer.engine_for(self).compile_pdf(plan.html, plan.css)
        except ValueError as error:
            renderer._raise_page_band_error(self, plan, error)
            raise
        renderer._check_layout(self, plan, prepared.export_render_time_pagination_trace())
        return prepared

    def render_pdf(self, renderer: Renderer, *, a11y_mode: str | None = None) -> bytes:
        return self.prepare(renderer, a11y_mode=a11y_mode).render_pdf()

    def render_image_pages(self, renderer: Renderer, *, dpi=150, a11y_mode: str | None = None) -> list[bytes]:
        return self.prepare(renderer, a11y_mode=a11y_mode).render_image_pages(dpi=dpi)

    def emit_artifacts(self, *, html_path, css_path, a11y_mode: str | None = None, target="html"):
        plan = self.compile(a11y_mode=a11y_mode, target=target)
        html_path, css_path = Path(html_path), Path(css_path)
        html_path.parent.mkdir(parents=True, exist_ok=True)
        css_path.parent.mkdir(parents=True, exist_ok=True)
        import os
        href = Path(os.path.relpath(css_path, html_path.parent)).as_posix()
        html = plan.html.replace("</head>", f'<link rel="stylesheet" href="{escape(href, quote=True)}"></head>', 1)
        html_path.write_text(html, encoding="utf-8")
        css_path.write_text(plan.css, encoding="utf-8")


@dataclass(frozen=True)
class _Closing:
    html: str


def _export_options(include_styles):
    if include_styles is not None and not isinstance(include_styles, bool):
        raise TypeError("include_styles must be True, False, or None")


def to_html(node_or_document, *, include_styles: bool | None = None,
            theme: Theme | None = None, a11y_mode: str | None = None) -> str:
    """Export a document or component fragment without creating a renderer.

    A fragment includes only its collected component CSS, not page geometry or
    document-wide text styles. Supply a theme to resolve the fragment's tokens.
    """
    _export_options(include_styles)
    if isinstance(node_or_document, Document):
        if theme is not None:
            raise AuthoringError("The document already owns its theme", code="document.theme")
        return node_or_document.to_html(include_styles=include_styles,
                                        a11y_mode=a11y_mode)
    source = Document(node_or_document, theme=theme)._compile(
        include_styles=include_styles is not False, a11y_mode=a11y_mode, fragment_only=True)
    return (style_element(source._component_css) if include_styles else "") + source._fragment


class Renderer:
    """Explicit engine configuration; each document gets isolated engine metadata.

    Font files, profiles, assets and diagnostic options remain caller choices.
    Each preparation creates a fresh engine and returns its owned layout snapshot.
    """

    def __init__(self, **engine_options):
        reserved = {"page_width", "page_height", "page_margins", "margin", "document_title", "document_lang"}
        if conflict := reserved.intersection(engine_options):
            raise AuthoringError(f"Document owns these settings: {', '.join(sorted(conflict))}", code="renderer.options")
        self._options = MappingProxyType(deepcopy(engine_options))

    def engine_for(self, document: Document):
        import fullbleed
        return fullbleed.PdfEngine(**deepcopy(dict(self._options)), document_title=document.title, document_lang=document.lang)

    def checked_engine_for(self, document: Document, plan: DocumentSource):
        """Compatibility helper; use Document.prepare to reuse the checked layout."""
        engine = self.engine_for(document)
        try:
            trace = engine.export_render_time_pagination_trace(plan.html, plan.css)
        except ValueError as error:
            self._raise_page_band_error(document, plan, error)
            raise
        self._check_layout(document, plan, trace)
        return engine

    @staticmethod
    def _raise_page_band_error(document, plan, error):
        marker = "page-margin layout: "
        if marker not in str(error):
            return
        try:
            details = json.loads(str(error).split(marker, 1)[1])
        except (ValueError, TypeError):
            return
        if details.get("schema") != "fullbleed.page_margin_fit.v1":
            return
        by_id = {source.node_id: source for source in plan.sources}
        sources = tuple(by_id[node_id] for node_id in details.get("source_ids", ()) if node_id in by_id)
        advice = {
            "paint_bounds_unknown": "Use a registered embedded font and supported paint operations; the engine cannot prove this content fits. "
                                    "Register fonts with Renderer(font_files=...) and select their family in Theme or the component's style.",
            "slot_outside_reserved_margin": "Keep this slot inside the margins configured on Page.",
            "nested_running_content": "Use ordinary components inside a PageBand; nested running elements are unsupported.",
            "paint_overflow": "Leave space around strokes, outlines and transformed content, or reduce the slot's content. "
                              "Conservative paint bounds must fit before the band can be placed.",
        }.get(details["reason"], "Reduce this slot's content or enlarge the corresponding Page margin.")
        check = details.get("paint_check", {}).get("reason")
        detail = f" ({check})" if check else ""
        raise PageBandError(
            f"Page {details['page']} @{details['slot']} could not fit its authored content{detail}. {advice}",
            details=details, sources=sources, path=(document.title, *sources[0].path) if sources else (document.title,),
        ) from error

    @staticmethod
    def _check_layout(document: Document, plan: DocumentSource, trace):
        for event in trace["events"]:
            if event.get("reason") == "forced_unsplittable_full_frame":
                by_id = {source.node_id: source for source in plan.sources}
                sources = tuple(by_id[node_id] for node_id in dict.fromkeys(event.get("source_ids", ()))
                                if node_id in by_id)
                failure = event.get("split_failure") or {}
                reason = failure.get("reason")
                failure_sources = tuple(by_id[node_id] for node_id in dict.fromkeys(failure.get("source_ids", ()))
                                        if node_id in by_id)
                path = (document.title, f"page {event['page']}")
                location = failure_sources or sources
                if len(location) == 1:
                    path += location[0].path
                advice = {
                    "clipped_container": "The container clips overflow and cannot paginate. Reduce its content or remove overflow clipping.",
                    "container_decoration_height": "Container margins, borders and padding leave no room for content. Reduce them or use a larger page.",
                    "table_rowspan": "Continuing an oversized table row with rowspans is unsupported. Restructure the spanning cells or make the table fit.",
                    "table_cell_alignment": "Oversized table cells can continue with top or baseline alignment. Set the affected cells' vertical alignment to top or baseline.",
                    "table_fixed_height": "The table's fixed rows and decorations leave no room for a body row. Reduce them or use a larger page.",
                    "table_cell_content": "The affected table cells cannot fit or make progress on this page. Resize unsplittable content or use content that can continue.",
                }.get(reason, "Restructure the block or use a larger page.")
                raise LayoutError(
                    "Content cannot fit or continue within the page. The engine would place an unsplittable "
                    "block beyond the page edge; no PDF was returned. " + advice,
                    page=event["page"], sources=sources, reason=reason, failure_sources=failure_sources, path=path,
                )
