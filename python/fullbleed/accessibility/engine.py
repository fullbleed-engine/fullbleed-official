from __future__ import annotations

import hashlib
import html as html_mod
import json
import re
import time
from pathlib import Path
from typing import Any

import fullbleed as _fullbleed

from .types import AccessibilityRunResult

_LETTER = ("8.5in", "11in")
_A4 = ("210mm", "297mm")
_TOKENS = (
    b"/StructTreeRoot",
    b"/StructElem",
    b"/MarkInfo",
    b"/Marked",
    b"/MCID",
    b"/Alt",
    b"/ActualText",
    b"/Figure",
    b"/Table",
    b"/TH",
    b"/TD",
    b"/TR",
)
_RE_MARKED_TRUE = re.compile(rb"/Marked\s+true\b")
_RE_LANG = re.compile(rb"/Lang\s*(\((?:\\.|[^\\)])*\)|<[^>]*>)")
_RE_TITLE = re.compile(rb"/Title\b")


def _dump_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def _coerce_int(value: Any) -> int | None:
    try:
        return None if value is None else int(value)
    except Exception:
        return None


def _first_not_none(*values: Any) -> Any:
    for value in values:
        if value is not None:
            return value
    return None


def _sha256_file(path: Path) -> str | None:
    try:
        return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
    except Exception:
        return None


def _normalize_css_href(css_href: str | None) -> str | None:
    if css_href is None:
        return None
    text = str(css_href).strip()
    return text or None


def _normalize_css_media(css_media: str | None) -> str | None:
    if css_media is None:
        return None
    text = str(css_media).strip()
    return text or None


def _inject_css_link(
    html_text: str,
    css_href: str | None,
    css_media: str | None = None,
) -> tuple[str, bool, bool]:
    href = _normalize_css_href(css_href)
    if not href:
        return html_text, False, False
    if "rel=\"stylesheet\"" in html_text or "rel='stylesheet'" in html_text:
        return html_text, False, True
    media_attr = ""
    media = _normalize_css_media(css_media)
    if media:
        media_attr = f' media="{html_mod.escape(media, quote=True)}"'
    link = (
        f'<link rel="stylesheet" href="{html_mod.escape(href, quote=True)}"'
        f"{media_attr} />"
    )
    if "</head>" in html_text:
        return html_text.replace("</head>", f"{link}</head>", 1), True, False
    return html_text, False, False


def _pdf_scan(pdf_bytes: bytes) -> dict[str, Any]:
    token_counts = {t.decode("ascii", "ignore").lstrip("/"): pdf_bytes.count(t) for t in _TOKENS}
    return {
        "bytes_len": len(pdf_bytes),
        "token_counts": token_counts,
        "struct_tree_root_present": token_counts.get("StructTreeRoot", 0) > 0,
        "mark_info_present": token_counts.get("MarkInfo", 0) > 0,
        "marked_true_present": bool(_RE_MARKED_TRUE.search(pdf_bytes)),
        "lang_token_present": bool(_RE_LANG.search(pdf_bytes)),
        "title_token_present": bool(_RE_TITLE.search(pdf_bytes)),
    }


def _reading_blocks_fitz(pdf_path: Path) -> tuple[list[dict[str, Any]], list[str], str]:
    warnings: list[str] = []
    try:
        import fitz  # type: ignore
    except Exception as exc:  # pragma: no cover
        return [], [f"fitz unavailable: {type(exc).__name__}: {exc}"], "none"
    pages: list[dict[str, Any]] = []
    try:
        doc = fitz.open(pdf_path)
        for page_index in range(int(doc.page_count)):
            page = doc.load_page(page_index)
            blocks = []
            for idx, block in enumerate(page.get_text("blocks") or []):
                if len(block) < 5:
                    continue
                x0, y0, x1, y1, text = block[:5]
                text = str(text or "").strip()
                if not text:
                    continue
                blocks.append({"index": idx, "bbox": [float(x0), float(y0), float(x1), float(y1)], "text": text})
            pages.append(
                {
                    "page_index": page_index,
                    "width": float(page.rect.width),
                    "height": float(page.rect.height),
                    "block_count": len(blocks),
                    "blocks": blocks,
                }
            )
        doc.close()
    except Exception as exc:  # pragma: no cover
        warnings.append(f"fitz extraction failed: {type(exc).__name__}: {exc}")
    return pages, warnings, ("fitz" if pages else "none")


def _reading_blocks_pypdf(pdf_path: Path) -> tuple[list[dict[str, Any]], list[str], str]:
    warnings: list[str] = []
    try:
        from pypdf import PdfReader  # type: ignore
    except Exception as exc:  # pragma: no cover
        return [], [f"pypdf unavailable: {type(exc).__name__}: {exc}"], "none"
    pages: list[dict[str, Any]] = []
    try:
        reader = PdfReader(str(pdf_path))
        for page_index, page in enumerate(reader.pages):
            lines = [ln.strip() for ln in (page.extract_text() or "").splitlines() if ln.strip()]
            blocks = [{"index": i, "text": ln} for i, ln in enumerate(lines)]
            pages.append({"page_index": page_index, "width": None, "height": None, "block_count": len(blocks), "blocks": blocks})
    except Exception as exc:  # pragma: no cover
        warnings.append(f"pypdf extraction failed: {type(exc).__name__}: {exc}")
    return pages, warnings, ("pypdf" if pages else "none")


def _contract_meta() -> dict[str, Any]:
    try:
        meta = _fullbleed.audit_contract_metadata()
    except Exception:
        return {}
    return dict(meta) if isinstance(meta, dict) else {}


class AccessibilityEngine:
    def __init__(
        self,
        *,
        page_size: str = "LETTER",
        document_lang: str | None = None,
        document_title: str | None = None,
        document_css_href: str | None = None,
        document_css_source_path: str | None = None,
        document_css_media: str | None = "all",
        document_css_required: bool | None = None,
        strict: bool = False,
        emit_reports_by_default: bool = True,
        render_previews_by_default: bool = True,
        **engine_kwargs: Any,
    ) -> None:
        if "pdf_profile" in engine_kwargs:
            raise TypeError("AccessibilityEngine does not accept pdf_profile (fixed to pdfua-targeted mode).")
        if not hasattr(_fullbleed, "PdfEngine"):
            raise RuntimeError("fullbleed.PdfEngine is unavailable in this environment")
        if "page_width" not in engine_kwargs and "page_height" not in engine_kwargs:
            key = str(page_size).strip().upper()
            if key == "LETTER":
                engine_kwargs["page_width"], engine_kwargs["page_height"] = _LETTER
            elif key == "A4":
                engine_kwargs["page_width"], engine_kwargs["page_height"] = _A4
        self._strict = bool(strict)
        self._emit_reports_by_default = bool(emit_reports_by_default)
        self._render_previews_by_default = bool(render_previews_by_default)
        self._document_css_href = _normalize_css_href(document_css_href)
        self._document_css_source_path = _normalize_css_href(document_css_source_path)
        self._document_css_media = _normalize_css_media(document_css_media)
        self._document_css_required = (
            bool(document_css_required) if document_css_required is not None else self._strict
        )
        self._last_css_link_result: dict[str, Any] = {
            "css_link_injected": False,
            "css_link_preexisting": False,
            "css_link_href": None,
            "css_link_media": None,
        }
        self._engine = _fullbleed.PdfEngine(
            pdf_profile="pdfua",
            document_lang=document_lang,
            document_title=document_title,
            **engine_kwargs,
        )
        for attr, value in (
            ("document_css_href", self._document_css_href),
            ("document_css_source_path", self._document_css_source_path),
            ("document_css_media", self._document_css_media),
            ("document_css_required", self._document_css_required),
        ):
            if hasattr(self._engine, attr):
                try:
                    setattr(self._engine, attr, value)
                except Exception:
                    pass

    def __getattr__(self, name: str) -> Any:
        return getattr(self._engine, name)

    @property
    def raw_engine(self):
        return self._engine

    @property
    def document_lang(self) -> str | None:
        return getattr(self._engine, "document_lang", None)

    @document_lang.setter
    def document_lang(self, value: str | None) -> None:
        self._engine.document_lang = value

    @property
    def document_title(self) -> str | None:
        return getattr(self._engine, "document_title", None)

    @document_title.setter
    def document_title(self, value: str | None) -> None:
        self._engine.document_title = value

    @property
    def document_css_href(self) -> str | None:
        if hasattr(self._engine, "document_css_href"):
            return _normalize_css_href(getattr(self._engine, "document_css_href", None))
        return _normalize_css_href(self._document_css_href)

    @document_css_href.setter
    def document_css_href(self, value: str | None) -> None:
        self._document_css_href = _normalize_css_href(value)
        if hasattr(self._engine, "document_css_href"):
            self._engine.document_css_href = self._document_css_href

    @property
    def document_css_source_path(self) -> str | None:
        if hasattr(self._engine, "document_css_source_path"):
            return _normalize_css_href(getattr(self._engine, "document_css_source_path", None))
        return _normalize_css_href(self._document_css_source_path)

    @document_css_source_path.setter
    def document_css_source_path(self, value: str | None) -> None:
        self._document_css_source_path = _normalize_css_href(value)
        if hasattr(self._engine, "document_css_source_path"):
            self._engine.document_css_source_path = self._document_css_source_path

    @property
    def document_css_media(self) -> str | None:
        if hasattr(self._engine, "document_css_media"):
            return _normalize_css_media(getattr(self._engine, "document_css_media", None))
        return _normalize_css_media(self._document_css_media)

    @document_css_media.setter
    def document_css_media(self, value: str | None) -> None:
        self._document_css_media = _normalize_css_media(value)
        if hasattr(self._engine, "document_css_media"):
            self._engine.document_css_media = self._document_css_media

    @property
    def document_css_required(self) -> bool:
        if hasattr(self._engine, "document_css_required"):
            try:
                return bool(getattr(self._engine, "document_css_required"))
            except Exception:
                return bool(self._document_css_required)
        return bool(self._document_css_required)

    @document_css_required.setter
    def document_css_required(self, value: bool) -> None:
        self._document_css_required = bool(value)
        if hasattr(self._engine, "document_css_required"):
            self._engine.document_css_required = bool(value)

    def document_metadata(self) -> dict[str, Any]:
        if hasattr(self._engine, "document_metadata"):
            meta: dict[str, Any] = dict(self._engine.document_metadata())
        else:
            meta = {
                "document_lang": self.document_lang,
                "document_title": self.document_title,
            }
        meta["document_css_href"] = self.document_css_href
        meta["document_css_source_path"] = self.document_css_source_path
        meta["document_css_media"] = self.document_css_media
        meta["document_css_required"] = bool(self.document_css_required)
        return meta

    def _metadata_warnings_or_raise(self) -> list[str]:
        meta = self.document_metadata()
        missing = [
            key
            for key in ("document_lang", "document_title")
            if not str(meta.get(key) or "").strip()
        ]
        warnings: list[str] = []
        if missing:
            warnings.append(
                "AccessibilityEngine metadata incomplete; engine defaults may apply for "
                + ", ".join(missing)
            )
        if bool(meta.get("document_css_required")) and not _normalize_css_href(
            meta.get("document_css_href")
        ):
            warnings.append(
                "CSS_METADATA_MISSING: document_css_required=True but document_css_href is missing."
            )
        source_path = _normalize_css_href(meta.get("document_css_source_path"))
        if source_path and not Path(source_path).exists():
            warnings.append(
                "CSS_METADATA_SOURCE_UNREADABLE: document_css_source_path does not exist."
            )
        if warnings and self._strict:
            raise ValueError("; ".join(warnings))
        return warnings

    def _effective_css_href(
        self,
        *,
        css_href: str | None = None,
        out_css_path: str | None = None,
    ) -> str | None:
        explicit = _normalize_css_href(css_href)
        if explicit:
            return explicit
        from_meta = _normalize_css_href(self.document_css_href)
        if from_meta:
            return from_meta
        if not out_css_path:
            return None
        basename = Path(out_css_path).name.strip()
        return basename or None

    def _record_css_link_result(
        self,
        *,
        css_link_injected: bool,
        css_link_preexisting: bool,
        css_link_href: str | None,
        css_link_media: str | None,
    ) -> None:
        self._last_css_link_result = {
            "css_link_injected": bool(css_link_injected),
            "css_link_preexisting": bool(css_link_preexisting),
            "css_link_href": _normalize_css_href(css_link_href),
            "css_link_media": _normalize_css_media(css_link_media),
        }

    def emit_html(
        self,
        body_html: str,
        out_html_path: str,
        *,
        css_href: str | None = None,
        css_media: str | None = None,
    ) -> str:
        self._metadata_warnings_or_raise()
        effective_css_href = self._effective_css_href(css_href=css_href)
        if self.document_css_required and not _normalize_css_href(self.document_css_href):
            raise ValueError(
                "CSS_METADATA_MISSING: document_css_required=True but document_css_href metadata is missing."
            )
        html_text = str(self._engine.emit_html(body_html, out_html_path, True))
        effective_css_media = _normalize_css_media(css_media) or self.document_css_media
        patched, injected, preexisting = _inject_css_link(
            html_text,
            effective_css_href,
            effective_css_media,
        )
        if patched != html_text:
            Path(out_html_path).write_text(patched, encoding="utf-8")
            html_text = patched
        self._record_css_link_result(
            css_link_injected=injected,
            css_link_preexisting=preexisting,
            css_link_href=effective_css_href,
            css_link_media=effective_css_media,
        )
        return html_text

    def emit_css(self, css_text: str | None, out_css_path: str | None = None) -> str:
        target = out_css_path or self.document_css_source_path
        if not target:
            raise ValueError(
                "emit_css requires out_css_path or document_css_source_path metadata."
            )
        if css_text is None:
            source_path = self.document_css_source_path
            if not source_path:
                raise ValueError(
                    "emit_css(css_text=None) requires document_css_source_path metadata."
                )
            css_text = Path(source_path).read_text(encoding="utf-8")
        return str(self._engine.emit_css(str(css_text), target))

    def emit_artifacts(
        self,
        body_html: str,
        css_text: str | None,
        out_html_path: str,
        out_css_path: str,
        *,
        css_href: str | None = None,
        css_media: str | None = None,
    ) -> dict[str, Any]:
        self._metadata_warnings_or_raise()
        if css_text is None:
            source_path = self.document_css_source_path
            if not source_path:
                raise ValueError(
                    "emit_artifacts(css_text=None) requires document_css_source_path metadata."
                )
            css_text = Path(source_path).read_text(encoding="utf-8")
        effective_css_href = self._effective_css_href(css_href=css_href, out_css_path=out_css_path)
        if self.document_css_required and not _normalize_css_href(self.document_css_href):
            raise ValueError(
                "CSS_METADATA_MISSING: document_css_required=True but document_css_href metadata is missing."
            )
        out = dict(
            self._engine.emit_artifacts(
                body_html,
                str(css_text),
                out_html_path,
                out_css_path,
                True,
            )
        )
        effective_css_media = _normalize_css_media(css_media) or self.document_css_media
        patched, injected, preexisting = _inject_css_link(
            str(out.get("html", "")),
            effective_css_href,
            effective_css_media,
        )
        if patched != out.get("html"):
            Path(out_html_path).write_text(patched, encoding="utf-8")
            out["html"] = patched
        out["css_link_injected"] = bool(injected)
        out["css_link_preexisting"] = bool(preexisting)
        out["css_link_href"] = effective_css_href
        out["css_link_media"] = effective_css_media
        self._record_css_link_result(
            css_link_injected=injected,
            css_link_preexisting=preexisting,
            css_link_href=effective_css_href,
            css_link_media=effective_css_media,
        )
        return out

    def verify_accessibility_artifacts(
        self,
        html_path: str,
        css_path: str,
        *,
        profile: str = "cav",
        mode: str = "error",
        a11y_report: dict[str, Any] | None = None,
        claim_evidence: dict[str, Any] | None = None,
        render_preview_png_path: str | None = None,
    ) -> dict[str, Any]:
        return dict(
            self._engine.verify_accessibility_artifacts(
                html_path,
                css_path,
                profile=profile,
                mode=mode,
                render_preview_png_path=render_preview_png_path,
                a11y_report=a11y_report,
                claim_evidence=claim_evidence,
            )
        )

    def _derive_pmr_kwargs(
        self,
        *,
        component_validation: dict[str, Any] | None = None,
        parity_report: dict[str, Any] | None = None,
        run_report: dict[str, Any] | None = None,
        source_analysis: dict[str, Any] | None = None,
        render_page_count: int | None = None,
    ) -> dict[str, Any]:
        component_validation = component_validation or {}
        parity_report = parity_report or {}
        run_report = run_report or {}
        source_analysis = source_analysis or {}
        parity_cov = parity_report.get("coverage") or {}
        parity_src = parity_report.get("source_characteristics") or {}
        run_metrics = run_report.get("metrics") or {}
        return {
            "overflow_count": _coerce_int(component_validation.get("overflow_count")),
            "known_loss_count": _coerce_int(component_validation.get("known_loss_count")),
            "source_page_count": _first_not_none(
                _coerce_int(source_analysis.get("page_count")),
                _coerce_int(parity_src.get("page_count")),
                _coerce_int(run_metrics.get("source_page_count")),
            ),
            "render_page_count": _first_not_none(
                render_page_count,
                _coerce_int(run_metrics.get("render_page_count")),
            ),
            "review_queue_items": _first_not_none(
                _coerce_int(parity_cov.get("review_queue_items")),
                _coerce_int(run_metrics.get("review_queue_items")),
            ),
        }

    def verify_pmr_artifacts(
        self,
        html_path: str,
        css_path: str,
        *,
        profile: str = "cav",
        mode: str = "error",
        component_validation: dict[str, Any] | None = None,
        parity_report: dict[str, Any] | None = None,
        run_report: dict[str, Any] | None = None,
        source_analysis: dict[str, Any] | None = None,
        render_page_count: int | None = None,
    ) -> dict[str, Any]:
        kwargs = self._derive_pmr_kwargs(
            component_validation=component_validation,
            parity_report=parity_report,
            run_report=run_report,
            source_analysis=source_analysis,
            render_page_count=render_page_count,
        )
        return dict(
            self._engine.verify_paged_media_rank_artifacts(
                html_path, css_path, profile=profile, mode=mode, **kwargs
            )
        )

    def export_reading_order_trace(self, pdf_path: str, *, out_path: str | None = None) -> dict[str, Any]:
        if hasattr(_fullbleed, "export_pdf_reading_order_trace"):
            trace = dict(_fullbleed.export_pdf_reading_order_trace(pdf_path))
            if out_path:
                _dump_json(Path(out_path), trace)
            return trace
        path = Path(pdf_path)
        pages, warnings, extractor = _reading_blocks_fitz(path)
        if not pages:
            pypdf_pages, pypdf_warnings, pypdf_extractor = _reading_blocks_pypdf(path)
            pages = pypdf_pages
            warnings.extend(pypdf_warnings)
            extractor = pypdf_extractor if pypdf_pages else extractor
        total_blocks = sum(int(page.get("block_count") or 0) for page in pages)
        trace = {
            "schema": "fullbleed.pdf.reading_order_trace.v1",
            "schema_version": 1,
            "seed_only": True,
            "pdf_path": str(path),
            "extractor": extractor,
            "ok": bool(pages),
            "pages": pages,
            "summary": {
                "page_count": len(pages),
                "total_blocks": total_blocks,
                "non_empty_pages": sum(1 for p in pages if (p.get("block_count") or 0) > 0),
            },
            "warnings": warnings,
            "generated_at_unix_ms": int(time.time() * 1000),
        }
        if out_path:
            _dump_json(Path(out_path), trace)
        return trace

    def export_pdf_structure_trace(self, pdf_path: str, *, out_path: str | None = None) -> dict[str, Any]:
        if hasattr(_fullbleed, "export_pdf_structure_trace"):
            trace = dict(_fullbleed.export_pdf_structure_trace(pdf_path))
            if out_path:
                _dump_json(Path(out_path), trace)
            return trace
        path = Path(pdf_path)
        warnings: list[str] = []
        try:
            pdf_bytes = path.read_bytes()
        except Exception as exc:
            warnings.append(f"failed to read pdf: {type(exc).__name__}: {exc}")
            pdf_bytes = b""
        scan = _pdf_scan(pdf_bytes)
        trace = {
            "schema": "fullbleed.pdf.structure_trace.v1",
            "schema_version": 1,
            "seed_only": True,
            "pdf_path": str(path),
            "extractor": "byte_scan",
            "ok": bool(pdf_bytes),
            "summary": {
                "bytes_len": scan["bytes_len"],
                "struct_tree_root_present": scan["struct_tree_root_present"],
                "mark_info_present": scan["mark_info_present"],
                "marked_true_present": scan["marked_true_present"],
                "lang_token_present": scan["lang_token_present"],
                "title_token_present": scan["title_token_present"],
            },
            "token_counts": scan["token_counts"],
            "warnings": warnings,
            "generated_at_unix_ms": int(time.time() * 1000),
        }
        if out_path:
            _dump_json(Path(out_path), trace)
        return trace

    def export_render_time_reading_order_trace(
        self,
        html: str,
        css: str,
        *,
        out_path: str | None = None,
    ) -> dict[str, Any]:
        if not hasattr(self._engine, "export_render_time_reading_order_trace"):
            raise AttributeError("PdfEngine.export_render_time_reading_order_trace is unavailable")
        trace = dict(self._engine.export_render_time_reading_order_trace(html, css))
        if out_path:
            _dump_json(Path(out_path), trace)
        return trace

    def export_render_time_structure_trace(
        self,
        html: str,
        css: str,
        *,
        out_path: str | None = None,
    ) -> dict[str, Any]:
        if not hasattr(self._engine, "export_render_time_structure_trace"):
            raise AttributeError("PdfEngine.export_render_time_structure_trace is unavailable")
        trace = dict(self._engine.export_render_time_structure_trace(html, css))
        if out_path:
            _dump_json(Path(out_path), trace)
        return trace

    def _build_pdf_ua_seed_report(
        self,
        pdf_path: str,
        *,
        mode: str = "error",
        reading_order_trace: dict[str, Any] | None = None,
        pdf_structure_trace: dict[str, Any] | None = None,
        reading_order_trace_render: dict[str, Any] | None = None,
        pdf_structure_trace_render: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        if (
            reading_order_trace is None
            and pdf_structure_trace is None
            and reading_order_trace_render is None
            and pdf_structure_trace_render is None
            and hasattr(_fullbleed, "verify_pdf_ua_seed")
        ):
            return dict(_fullbleed.verify_pdf_ua_seed(pdf_path, mode=mode))
        path = Path(pdf_path)
        warnings: list[str] = []
        try:
            pdf_bytes = path.read_bytes()
        except Exception as exc:
            warnings.append(f"failed to read pdf: {type(exc).__name__}: {exc}")
            pdf_bytes = b""
        scan = _pdf_scan(pdf_bytes)
        ro = reading_order_trace or self.export_reading_order_trace(str(path))
        st = pdf_structure_trace or self.export_pdf_structure_trace(str(path))
        ro_render = reading_order_trace_render
        st_render = pdf_structure_trace_render
        ro_ok = bool((ro.get("summary") or {}).get("total_blocks"))
        st_ok = bool((st.get("summary") or {}).get("struct_tree_root_present"))
        ro_render_ok = bool((ro_render or {}).get("summary", {}).get("total_blocks"))
        st_render_ok = bool((st_render or {}).get("summary", {}).get("struct_tree_root_present"))

        checks = [
            {
                "id": "pdf.mark_info.present",
                "verdict": "pass" if scan["mark_info_present"] else "fail",
                "severity": "error",
                "critical": True,
                "message": "PDF MarkInfo token present" if scan["mark_info_present"] else "PDF MarkInfo token not found",
            },
            {
                "id": "pdf.mark_info.marked_true",
                "verdict": "pass" if scan["marked_true_present"] else "fail",
                "severity": "error",
                "critical": True,
                "message": "PDF /Marked true present" if scan["marked_true_present"] else "PDF /Marked true not found",
            },
            {
                "id": "pdf.structure_root.present",
                "verdict": "pass" if scan["struct_tree_root_present"] else "fail",
                "severity": "error",
                "critical": True,
                "message": "PDF StructTreeRoot token present" if scan["struct_tree_root_present"] else "PDF StructTreeRoot token not found",
            },
            {
                "id": "pdf.catalog.lang.present_seed",
                "verdict": "pass" if scan["lang_token_present"] else "warn",
                "severity": "warn",
                "critical": False,
                "message": "PDF /Lang token present" if scan["lang_token_present"] else "PDF /Lang token not found by seed byte scan",
            },
            {
                "id": "pdf.metadata.title.present_seed",
                "verdict": "pass" if scan["title_token_present"] else "warn",
                "severity": "warn",
                "critical": False,
                "message": "PDF /Title token present" if scan["title_token_present"] else "PDF /Title token not found by seed byte scan",
            },
            {
                "id": "pdf.trace.reading_order.emitted",
                "verdict": "pass" if ro_ok else "manual_needed",
                "severity": "warn",
                "critical": False,
                "message": "Reading-order trace contains extractable blocks" if ro_ok else "Reading-order trace emitted but extraction was empty; manual verification required",
                "evidence": {
                    "extractor": ro.get("extractor"),
                    "page_count": (ro.get("summary") or {}).get("page_count"),
                    "total_blocks": (ro.get("summary") or {}).get("total_blocks"),
                },
            },
            {
                "id": "pdf.trace.structure.emitted",
                "verdict": "pass" if st_ok else "manual_needed",
                "severity": "warn",
                "critical": False,
                "message": "Structure trace indicates tagged tokens" if st_ok else "Structure trace emitted but tag tokens were not observed; manual verification required",
                "evidence": {
                    "extractor": st.get("extractor"),
                    "struct_tree_root_present": (st.get("summary") or {}).get("struct_tree_root_present"),
                    "marked_true_present": (st.get("summary") or {}).get("marked_true_present"),
                },
            },
        ]
        if ro_render is not None:
            checks.append(
                {
                    "id": "pdf.trace.reading_order.render_time.emitted",
                    "verdict": "pass" if ro_render_ok else "manual_needed",
                    "severity": "warn",
                    "critical": False,
                    "message": "Render-time reading-order trace contains blocks"
                    if ro_render_ok
                    else "Render-time reading-order trace emitted but empty; manual verification required",
                    "evidence": {
                        "extractor": ro_render.get("extractor"),
                        "page_count": (ro_render.get("summary") or {}).get("page_count"),
                        "total_blocks": (ro_render.get("summary") or {}).get("total_blocks"),
                    },
                }
            )
        if st_render is not None:
            checks.append(
                {
                    "id": "pdf.trace.structure.render_time.emitted",
                    "verdict": "pass" if st_render_ok else "manual_needed",
                    "severity": "warn",
                    "critical": False,
                    "message": "Render-time structure trace indicates tagging activity"
                    if st_render_ok
                    else "Render-time structure trace emitted but no tags observed; manual verification required",
                    "evidence": {
                        "extractor": st_render.get("extractor"),
                        "struct_tree_root_present": (st_render.get("summary") or {}).get(
                            "struct_tree_root_present"
                        ),
                        "begin_tag_count": (st_render.get("summary") or {}).get("begin_tag_count"),
                    },
                }
            )
            st_render_summary = st_render.get("summary") or {}
            begin_tag_count = int(st_render_summary.get("begin_tag_count") or 0)
            end_tag_count = int(st_render_summary.get("end_tag_count") or 0)
            tag_balance_ok = bool(st_render_summary.get("tag_balance_ok"))
            tag_balance_underflow_count = int(st_render_summary.get("tag_balance_underflow_count") or 0)
            tagged_text_draw_count = int(st_render_summary.get("tagged_text_draw_count") or 0)
            untagged_text_draw_count = int(st_render_summary.get("untagged_text_draw_count") or 0)
            total_text_draw_count = tagged_text_draw_count + untagged_text_draw_count
            untagged_ratio = (
                (untagged_text_draw_count / total_text_draw_count) if total_text_draw_count > 0 else 0.0
            )
            checks.append(
                {
                    "id": "pdf.trace.structure.render_time.tag_balance_seed",
                    "verdict": "pass" if tag_balance_ok and begin_tag_count == end_tag_count else "warn",
                    "severity": "warn",
                    "critical": False,
                    "message": (
                        "Render-time structure tag stack remained balanced"
                        if tag_balance_ok and begin_tag_count == end_tag_count
                        else "Render-time structure tag stack imbalance detected"
                    ),
                    "evidence": {
                        "begin_tag_count": begin_tag_count,
                        "end_tag_count": end_tag_count,
                        "tag_balance_ok": tag_balance_ok,
                        "tag_balance_underflow_count": tag_balance_underflow_count,
                    },
                }
            )
            checks.append(
                {
                    "id": "pdf.trace.structure.render_time.tagged_text_presence_seed",
                    "verdict": (
                        "pass"
                        if tagged_text_draw_count > 0
                        else ("manual_needed" if total_text_draw_count > 0 else "not_applicable")
                    ),
                    "severity": "warn",
                    "critical": False,
                    "message": (
                        "Render-time structure trace includes tagged text draws"
                        if tagged_text_draw_count > 0
                        else (
                            "No tagged text draws observed despite text draws; manual verification required"
                            if total_text_draw_count > 0
                            else "No text draws observed in render-time structure trace"
                        )
                    ),
                    "evidence": {
                        "tagged_text_draw_count": tagged_text_draw_count,
                        "untagged_text_draw_count": untagged_text_draw_count,
                        "total_text_draw_count": total_text_draw_count,
                    },
                }
            )
            # v1 heuristic only: high untagged ratio is a risk indicator, not a conformance verdict.
            ratio_verdict = "pass"
            ratio_message = "Render-time tagged/untagged text ratio is within v1 seed tolerance"
            if total_text_draw_count >= 20:
                if untagged_ratio >= 0.95:
                    ratio_verdict = "warn"
                    ratio_message = "Very high untagged text ratio observed in render-time trace"
                elif untagged_ratio >= 0.80:
                    ratio_verdict = "manual_needed"
                    ratio_message = "High untagged text ratio observed; manual PDF tag verification recommended"
            checks.append(
                {
                    "id": "pdf.trace.structure.render_time.untagged_text_ratio_seed",
                    "verdict": ratio_verdict,
                    "severity": "warn",
                    "critical": False,
                    "message": ratio_message,
                    "evidence": {
                        "tagged_text_draw_count": tagged_text_draw_count,
                        "untagged_text_draw_count": untagged_text_draw_count,
                        "total_text_draw_count": total_text_draw_count,
                        "untagged_ratio": round(untagged_ratio, 4),
                        "warn_ratio": 0.95,
                        "manual_ratio": 0.80,
                        "minimum_total_text_draw_count": 20,
                    },
                }
            )
        if ro_render is not None:
            render_blocks = int((ro_render.get("summary") or {}).get("total_blocks") or 0)
            pdf_blocks = int((ro.get("summary") or {}).get("total_blocks") or 0)
            larger = max(render_blocks, pdf_blocks, 1)
            delta_ratio = abs(render_blocks - pdf_blocks) / larger
            if render_blocks == 0 and pdf_blocks == 0:
                verdict = "manual_needed"
                message = "Both reading-order traces are empty; manual verification required"
            elif render_blocks == 0 or pdf_blocks == 0:
                verdict = "warn"
                message = "Reading-order trace cross-check mismatch: one extractor returned no blocks"
            elif delta_ratio > 0.25:
                verdict = "warn"
                message = "Reading-order trace cross-check block counts diverge materially"
            else:
                verdict = "pass"
                message = "Reading-order trace cross-check is within v1 seed tolerance"
            checks.append(
                {
                    "id": "pdf.trace.reading_order.cross_check_seed",
                    "verdict": verdict,
                    "severity": "warn",
                    "critical": False,
                    "message": message,
                    "evidence": {
                        "render_extractor": ro_render.get("extractor"),
                        "render_total_blocks": render_blocks,
                        "pdf_extractor": ro.get("extractor"),
                        "pdf_total_blocks": pdf_blocks,
                        "delta_ratio": round(delta_ratio, 4),
                        "tolerance_ratio": 0.25,
                    },
                }
            )
        if st_render is not None:
            render_tagged = bool((st_render.get("summary") or {}).get("struct_tree_root_present"))
            pdf_tagged = bool((st.get("summary") or {}).get("struct_tree_root_present"))
            if render_tagged == pdf_tagged:
                verdict = "pass"
                message = "Structure trace cross-check agrees on tagged structure presence"
            else:
                verdict = "warn"
                message = "Structure trace cross-check disagrees on tagged structure presence"
            checks.append(
                {
                    "id": "pdf.trace.structure.cross_check_seed",
                    "verdict": verdict,
                    "severity": "warn",
                    "critical": False,
                    "message": message,
                    "evidence": {
                        "render_extractor": (st_render or {}).get("extractor"),
                        "render_struct_tree_root_present": render_tagged,
                        "pdf_extractor": st.get("extractor"),
                        "pdf_struct_tree_root_present": pdf_tagged,
                    },
                }
            )
        critical_fail_count = sum(1 for c in checks if c["critical"] and c["verdict"] == "fail")
        gate_ok = critical_fail_count == 0 or str(mode).lower() != "error"
        report = {
            "schema": "fullbleed.pdf.ua_seed_verify.v1",
            "schema_version": 1,
            "seed_only": True,
            "pdf_path": str(path),
            "mode": mode,
            "ok": gate_ok,
            "gate": {
                "ok": gate_ok,
                "critical_fail_count": critical_fail_count,
                "nonpass_count": sum(1 for c in checks if c["verdict"] in {"fail", "warn", "manual_needed"}),
                "mode": mode,
            },
            "checks": checks,
            "cross_checks": {
                "reading_order": {
                    "render_extractor": (ro_render or {}).get("extractor"),
                    "render_total_blocks": ((ro_render or {}).get("summary") or {}).get(
                        "total_blocks"
                    ),
                    "pdf_extractor": ro.get("extractor"),
                    "pdf_total_blocks": (ro.get("summary") or {}).get("total_blocks"),
                }
                if ro_render is not None
                else None,
                "structure": {
                    "render_extractor": (st_render or {}).get("extractor"),
                    "render_tagged_present": ((st_render or {}).get("summary") or {}).get(
                        "struct_tree_root_present"
                    ),
                    "pdf_extractor": st.get("extractor"),
                    "pdf_tagged_present": (st.get("summary") or {}).get("struct_tree_root_present"),
                }
                if st_render is not None
                else None,
            },
            "warnings": warnings,
            "generated_at_unix_ms": int(time.time() * 1000),
        }
        meta = _contract_meta()
        if meta:
            report["tooling"] = {
                "audit_contract_id": meta.get("contract_id"),
                "audit_contract_version": meta.get("contract_version"),
                "audit_contract_fingerprint": meta.get("contract_fingerprint"),
            }
        return report

    def verify_pdf_ua_seed_artifacts(self, pdf_path: str, *, mode: str = "error") -> dict[str, Any]:
        return self._build_pdf_ua_seed_report(pdf_path, mode=mode)

    def _emit_preview_pngs(self, html: str, css: str, out_dir: Path, *, stem: str) -> list[str]:
        if hasattr(self._engine, "render_image_pages_to_dir"):
            return list(self._engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
        if hasattr(self._engine, "render_image_pages"):
            out: list[str] = []
            for idx, image_bytes in enumerate(self._engine.render_image_pages(html, css, 144) or [], start=1):
                path = out_dir / f"{stem}_page{idx}.png"
                path.write_bytes(image_bytes)
                out.append(str(path))
            return out
        return []

    def render_bundle(
        self,
        *,
        body_html: str,
        css_text: str,
        out_dir: str,
        stem: str,
        profile: str = "cav",
        a11y_mode: str | None = "raise",
        a11y_report: dict[str, Any] | None = None,
        claim_evidence: dict[str, Any] | None = None,
        component_validation: dict[str, Any] | None = None,
        parity_report: dict[str, Any] | None = None,
        source_analysis: dict[str, Any] | None = None,
        render_preview_png: bool | None = None,
        run_verifier: bool | None = None,
        run_pmr: bool | None = None,
        run_pdf_ua_seed_verify: bool | None = None,
        emit_reading_order_trace: bool | None = None,
        emit_pdf_structure_trace: bool | None = None,
    ) -> AccessibilityRunResult:
        warnings = self._metadata_warnings_or_raise()
        out_dir_path = Path(out_dir)
        out_dir_path.mkdir(parents=True, exist_ok=True)
        render_preview_png = self._render_previews_by_default if render_preview_png is None else bool(render_preview_png)
        default_reports = self._emit_reports_by_default
        run_verifier = default_reports if run_verifier is None else bool(run_verifier)
        run_pmr = default_reports if run_pmr is None else bool(run_pmr)
        run_pdf_ua_seed_verify = default_reports if run_pdf_ua_seed_verify is None else bool(run_pdf_ua_seed_verify)
        emit_reading_order_trace = True if emit_reading_order_trace is None else bool(emit_reading_order_trace)
        emit_pdf_structure_trace = True if emit_pdf_structure_trace is None else bool(emit_pdf_structure_trace)

        html_path = out_dir_path / f"{stem}.html"
        css_path = out_dir_path / f"{stem}.css"
        pdf_path = out_dir_path / f"{stem}.pdf"
        a11y_path = out_dir_path / f"{stem}_a11y_verify_engine.json"
        pmr_path = out_dir_path / f"{stem}_pmr_engine.json"
        pdf_ua_seed_path = out_dir_path / f"{stem}_pdf_ua_seed_verify.json"
        reading_path = out_dir_path / f"{stem}_reading_order_trace.json"
        structure_path = out_dir_path / f"{stem}_pdf_structure_trace.json"
        reading_render_path = out_dir_path / f"{stem}_reading_order_trace_render.json"
        structure_render_path = out_dir_path / f"{stem}_pdf_structure_trace_render.json"
        run_report_path = out_dir_path / f"{stem}_run_report.json"

        emitted = self.emit_artifacts(body_html, css_text, str(html_path), str(css_path))
        html_text = str(emitted.get("html", ""))
        css_out = str(emitted.get("css", css_text))
        css_link_href = _normalize_css_href(emitted.get("css_link_href"))
        css_link_media = _normalize_css_media(emitted.get("css_link_media"))
        css_link_injected = bool(emitted.get("css_link_injected", False))
        css_link_preexisting = bool(emitted.get("css_link_preexisting", False))
        # Render from the authored fragment/body + CSS, not the emitted HTML artifact, so the
        # injected <link rel="stylesheet"> in the artifact does not create engine asset warnings.
        pdf_bytes = int(self._engine.render_pdf_to_file(body_html, css_out, str(pdf_path)))
        png_paths = self._emit_preview_pngs(body_html, css_out, out_dir_path, stem=stem) if render_preview_png else []

        verifier_report: dict[str, Any] | None = None
        pmr_report: dict[str, Any] | None = None
        pdf_ua_seed_report: dict[str, Any] | None = None
        reading_trace: dict[str, Any] | None = None
        structure_trace: dict[str, Any] | None = None
        reading_trace_render: dict[str, Any] | None = None
        structure_trace_render: dict[str, Any] | None = None

        if run_verifier and hasattr(self._engine, "verify_accessibility_artifacts"):
            contrast_png = png_paths[0] if png_paths else None
            try:
                verifier_report = self.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile=profile,
                    mode="error",
                    render_preview_png_path=contrast_png,
                    a11y_report=a11y_report,
                    claim_evidence=claim_evidence,
                )
            except TypeError:
                warnings.append("Engine verifier compatibility fallback used (missing newer verifier hooks).")
                verifier_report = dict(
                    self._engine.verify_accessibility_artifacts(str(html_path), str(css_path), profile=profile, mode="error")
                )
            _dump_json(a11y_path, verifier_report)

        if run_pmr and hasattr(self._engine, "verify_paged_media_rank_artifacts"):
            try:
                pmr_report = self.verify_pmr_artifacts(
                    str(html_path),
                    str(css_path),
                    profile=profile,
                    mode="error",
                    component_validation=component_validation,
                    parity_report=parity_report,
                    source_analysis=source_analysis,
                    render_page_count=len(png_paths),
                )
            except TypeError:
                warnings.append("Engine PMR compatibility fallback used (sidecar-derived counts not applied).")
                pmr_report = dict(
                    self._engine.verify_paged_media_rank_artifacts(str(html_path), str(css_path), profile=profile, mode="error")
                )
            _dump_json(pmr_path, pmr_report)

        if emit_reading_order_trace and hasattr(self._engine, "export_render_time_reading_order_trace"):
            try:
                reading_trace_render = self.export_render_time_reading_order_trace(
                    body_html,
                    css_out,
                    out_path=str(reading_render_path),
                )
            except Exception as exc:
                warnings.append(
                    f"render-time reading-order trace unavailable: {type(exc).__name__}: {exc}"
                )
        if emit_pdf_structure_trace and hasattr(self._engine, "export_render_time_structure_trace"):
            try:
                structure_trace_render = self.export_render_time_structure_trace(
                    body_html,
                    css_out,
                    out_path=str(structure_render_path),
                )
            except Exception as exc:
                warnings.append(
                    f"render-time structure trace unavailable: {type(exc).__name__}: {exc}"
                )

        if emit_reading_order_trace:
            reading_trace = self.export_reading_order_trace(str(pdf_path), out_path=str(reading_path))
        if emit_pdf_structure_trace:
            structure_trace = self.export_pdf_structure_trace(str(pdf_path), out_path=str(structure_path))
        if run_pdf_ua_seed_verify:
            pdf_ua_seed_report = self._build_pdf_ua_seed_report(
                str(pdf_path),
                mode="error",
                reading_order_trace=reading_trace,
                pdf_structure_trace=structure_trace,
                reading_order_trace_render=reading_trace_render,
                pdf_structure_trace_render=structure_trace_render,
            )
            _dump_json(pdf_ua_seed_path, pdf_ua_seed_report)

        meta = _contract_meta()
        verifier_ok = bool((verifier_report or {}).get("gate", {}).get("ok", True))
        pmr_ok = bool((pmr_report or {}).get("gate", {}).get("ok", True))
        seed_ok = bool((pdf_ua_seed_report or {}).get("gate", {}).get("ok", True))
        ok = verifier_ok and pmr_ok and seed_ok
        run_report = {
            "schema": "fullbleed.accessibility.run_bundle.v1",
            "pdf_ua_targeted": True,
            "engine_pdf_profile_requested": "pdfua",
            "engine_pdf_profile_effective": "tagged",
            "document_lang": self.document_metadata().get("document_lang"),
            "document_title": self.document_metadata().get("document_title"),
            "document_css_href": self.document_metadata().get("document_css_href"),
            "document_css_source_path": self.document_metadata().get("document_css_source_path"),
            "document_css_media": self.document_metadata().get("document_css_media"),
            "document_css_required": bool(
                self.document_metadata().get("document_css_required")
            ),
            "profile": profile,
            "a11y_mode": a11y_mode,
            "ok": ok,
            "html_path": str(html_path),
            "css_path": str(css_path),
            "pdf_path": str(pdf_path),
            "pdf_ua_seed_verify_path": str(pdf_ua_seed_path) if pdf_ua_seed_report else None,
            "reading_order_trace_path": str(reading_path) if reading_trace else None,
            "pdf_structure_trace_path": str(structure_path) if structure_trace else None,
            "reading_order_trace_render_path": str(reading_render_path)
            if reading_trace_render
            else None,
            "pdf_structure_trace_render_path": str(structure_render_path)
            if structure_trace_render
            else None,
            "render_preview_png_paths": png_paths,
            "engine_a11y_verify_path": str(a11y_path) if verifier_report else None,
            "engine_pmr_path": str(pmr_path) if pmr_report else None,
            "engine_a11y_verify_ok": verifier_ok if verifier_report else None,
            "engine_pmr_ok": pmr_ok if pmr_report else None,
            "engine_pmr_score": ((pmr_report or {}).get("rank") or {}).get("score"),
            "pdf_ua_seed_ok": seed_ok if pdf_ua_seed_report else None,
            "css_link_href": css_link_href,
            "css_link_media": css_link_media,
            "css_link_injected": css_link_injected,
            "css_link_preexisting": css_link_preexisting,
            "audit_contract_fingerprint": meta.get("contract_fingerprint"),
            "audit_registry_hash": meta.get("audit_registry_hash"),
            "wcag20aa_registry_hash": meta.get("wcag20aa_registry_hash"),
            "section508_html_registry_hash": meta.get("section508_html_registry_hash"),
            "pdf_sha256": _sha256_file(pdf_path),
            "metrics": {
                "pdf_bytes": pdf_bytes,
                "render_page_count": len(png_paths),
                "source_page_count": _coerce_int((source_analysis or {}).get("page_count")),
                "overflow_count": _coerce_int((component_validation or {}).get("overflow_count")),
                "known_loss_count": _coerce_int((component_validation or {}).get("known_loss_count")),
                "css_link_injected": css_link_injected,
                "css_link_preexisting": css_link_preexisting,
            },
            "warnings": warnings,
        }
        if reading_trace_render is not None and reading_trace is not None:
            run_report["reading_order_trace_cross_check"] = {
                "render_extractor": reading_trace_render.get("extractor"),
                "render_total_blocks": (reading_trace_render.get("summary") or {}).get(
                    "total_blocks"
                ),
                "pdf_extractor": reading_trace.get("extractor"),
                "pdf_total_blocks": (reading_trace.get("summary") or {}).get("total_blocks"),
            }
        if structure_trace_render is not None and structure_trace is not None:
            run_report["pdf_structure_trace_cross_check"] = {
                "render_extractor": structure_trace_render.get("extractor"),
                "render_tagged_present": (structure_trace_render.get("summary") or {}).get(
                    "struct_tree_root_present"
                ),
                "pdf_extractor": structure_trace.get("extractor"),
                "pdf_tagged_present": (structure_trace.get("summary") or {}).get(
                    "struct_tree_root_present"
                ),
            }
        _dump_json(run_report_path, run_report)

        paths = {
            "html_path": str(html_path),
            "css_path": str(css_path),
            "pdf_path": str(pdf_path),
            "run_report_path": str(run_report_path),
        }
        if verifier_report:
            paths["engine_a11y_verify_path"] = str(a11y_path)
        if pmr_report:
            paths["engine_pmr_path"] = str(pmr_path)
        if pdf_ua_seed_report:
            paths["pdf_ua_seed_verify_path"] = str(pdf_ua_seed_path)
        if reading_trace:
            paths["reading_order_trace_path"] = str(reading_path)
        if structure_trace:
            paths["pdf_structure_trace_path"] = str(structure_path)
        if reading_trace_render:
            paths["reading_order_trace_render_path"] = str(reading_render_path)
        if structure_trace_render:
            paths["pdf_structure_trace_render_path"] = str(structure_render_path)

        return AccessibilityRunResult(
            ok=ok,
            pdf_ua_targeted=True,
            paths=paths,
            verifier_report=verifier_report,
            pmr_report=pmr_report,
            pdf_ua_seed_report=pdf_ua_seed_report,
            reading_order_trace=reading_trace,
            pdf_structure_trace=structure_trace,
            reading_order_trace_render=reading_trace_render,
            pdf_structure_trace_render=structure_trace_render,
            run_report=run_report,
            contract_fingerprint=meta.get("contract_fingerprint"),
            warnings=warnings,
        )
