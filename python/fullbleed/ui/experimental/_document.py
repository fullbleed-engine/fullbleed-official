# SPDX-License-Identifier: MIT
from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha256
from html import escape
from pathlib import Path
from types import MappingProxyType
from copy import deepcopy

from ..core import _render_attrs, _HTML_VOID_ELEMENTS
from ._style import AuthoringError, Length, Style, Theme, mm
from ._tree import Node, _children, _required_tokens


@dataclass(frozen=True)
class Page:
    """Page geometry, compiled to an actual @page rule."""

    size: str | tuple[Length, Length] = "A4"
    margin: Length | tuple[Length, ...] = mm(18)
    landscape: bool = False

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


@dataclass(frozen=True)
class CompiledDocument:
    html: str
    css: str
    sources: tuple[Source, ...]


@dataclass(frozen=True, init=False)
class Document:
    """A frozen document value. Compilation needs no renderer or global registry."""

    children: tuple
    title: str
    lang: str
    page: Page
    theme: Theme

    def __init__(self, *children, title="Fullbleed document", lang="en", page=None, theme=None):
        if not isinstance(title, str) or not title.strip() or not isinstance(lang, str) or not lang.strip():
            raise AuthoringError("Document title and language must be nonempty strings", code="document.metadata")
        if page is not None and not isinstance(page, Page) or theme is not None and not isinstance(theme, Theme):
            raise AuthoringError("Document expects Page and Theme values", code="document.options")
        for name, value in {"children": _children(children), "title": title, "lang": lang,
                            "page": page or Page(), "theme": theme or Theme()}.items():
            object.__setattr__(self, name, value)

    def compile(self) -> CompiledDocument:
        rules, sources, output = {}, [], []
        # Explicit traversal avoids Python recursion limits for nested documents.
        pending = [(child, (), None) for child in reversed(self.children)]
        while pending:
            node, path, source = pending.pop()
            if isinstance(node, _Closing):
                output.append(node.html)
                continue
            if isinstance(node, str):
                output.append(escape(node))
                continue
            if node.name is not None:
                path = (*path, node.name)
                source = node.source or source
            if node.tag is None:
                pending.extend((child, path, source) for child in reversed(node.children))
                continue
            node_id = f"n{len(sources) + 1}"
            location = (*path, node.part or node.tag)
            sources.append(Source(node_id, location, source[0] if source else None, source[1] if source else None))
            attributes = dict(node.attributes)
            attributes["data-fb-id"] = node_id
            try:
                declarations = node._effective_style().resolve(self.theme)
            except AuthoringError as error:
                raise AuthoringError(error.message, code=error.code, path=location) from error
            if declarations:
                class_name = "fbx-" + sha256(declarations.encode()).hexdigest()[:24]
                if class_name in rules and rules[class_name] != declarations:
                    raise AuthoringError("Generated style identity collision", code="style.collision")
                rules[class_name] = declarations
                attributes["class"] = " ".join(filter(None, (attributes.get("class"), class_name)))
            output.append(f"<{node.tag}{_render_attrs(attributes)}>")
            if node.tag.casefold() not in _HTML_VOID_ELEMENTS:
                pending.append((_Closing(f"</{node.tag}>"), path, source))
                pending.extend((child, path, source) for child in reversed(node.children))
        body = Style(margin=0, box_sizing="border-box")
        text = Style(body, self.theme.text).resolve(self.theme)
        css = "\n".join((self.page.to_css(), f"body {{ {text} }}",
                          "* { box-sizing: border-box; }",
                          *(f".{name} {{ {value} }}" for name, value in rules.items())))
        html = (f'<!doctype html><html lang="{escape(self.lang, quote=True)}"><head>'
                f'<meta charset="utf-8"><title>{escape(self.title)}</title></head><body><main>'
                + "".join(output) + "</main></body></html>")
        return CompiledDocument(html, css, tuple(sources))

    @property
    def required_tokens(self) -> tuple[str, ...]:
        """Sorted explicit token names used by content and the theme's text style."""
        return tuple(sorted(set(_required_tokens(self.children)) | set(self.theme.text._token_names())))

    def to_html(self) -> str:
        return self.compile().html

    def to_css(self) -> str:
        return self.compile().css

    def prepare(self, renderer: Renderer):
        """Lay out and check once, returning the engine's owned CompiledDocument.

        Reuse the result for PDF bytes, layout previews and pagination diagnostics.
        A changed document or renderer needs a new preparation. No global cache
        or mutable engine is shared between documents.
        """
        plan = self.compile()
        prepared = renderer.engine_for(self).compile_pdf(plan.html, plan.css)
        renderer._check_layout(self, plan, prepared.export_render_time_pagination_trace())
        return prepared

    def render_pdf(self, renderer: Renderer) -> bytes:
        return self.prepare(renderer).render_pdf()

    def render_image_pages(self, renderer: Renderer, *, dpi=150) -> list[bytes]:
        return self.prepare(renderer).render_image_pages(dpi=dpi)

    def emit_artifacts(self, *, html_path, css_path):
        plan = self.compile()
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

    def checked_engine_for(self, document: Document, plan: CompiledDocument):
        """Compatibility helper; use Document.prepare to reuse the checked layout."""
        engine = self.engine_for(document)
        trace = engine.export_render_time_pagination_trace(plan.html, plan.css)
        self._check_layout(document, plan, trace)
        return engine

    @staticmethod
    def _check_layout(document: Document, plan: CompiledDocument, trace):
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
                    "table_cell_alignment": "Oversized table cells require top alignment to continue. Set the affected cells' vertical alignment to top.",
                    "table_fixed_height": "The table's fixed rows and decorations leave no room for a body row. Reduce them or use a larger page.",
                    "table_cell_content": "The affected table cells cannot fit or make progress on this page. Resize unsplittable content or use content that can continue.",
                }.get(reason, "Restructure the block or use a larger page.")
                raise LayoutError(
                    "Content cannot fit or continue within the page. The engine would place an unsplittable "
                    "block beyond the page edge; no PDF was returned. " + advice,
                    page=event["page"], sources=sources, reason=reason, failure_sources=failure_sources, path=path,
                )
