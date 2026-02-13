from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any, Callable

from components.fb_ui import DocumentArtifact, compile_document


@dataclass
class Diagnostic:
    code: str
    level: str
    message: str
    data: dict[str, Any] = field(default_factory=dict)


@dataclass
class RenderValidation:
    ok: bool
    bytes_written: int
    page_count: int | None
    diagnostics: list[Diagnostic] = field(default_factory=list)
    checks: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "bytes_written": self.bytes_written,
            "page_count": self.page_count,
            "checks": self.checks,
            "diagnostics": [asdict(item) for item in self.diagnostics],
        }


def validate_render(
    *,
    engine: Any,
    component: Callable[[], DocumentArtifact],
    css: str,
    expected_page_count: int | None = 1,
) -> RenderValidation:
    artifact = component()
    html = compile_document(artifact)

    diagnostics: list[Diagnostic] = []
    checks: dict[str, Any] = {
        "component": component.__name__,
        "page_target": artifact.page,
        "bootstrap_hint": artifact.bootstrap,
    }

    try:
        pdf_bytes, glyph_report = engine.render_pdf_with_glyph_report(html, css)
    except Exception as exc:
        diagnostics.append(
            Diagnostic(
                code="FB_RENDER_EXCEPTION",
                level="error",
                message="Engine render raised an exception.",
                data={"exception": repr(exc)},
            )
        )
        return RenderValidation(ok=False, bytes_written=0, page_count=None, diagnostics=diagnostics, checks=checks)

    byte_len = len(pdf_bytes)
    checks["byte_length_positive"] = byte_len > 0

    if byte_len <= 0:
        diagnostics.append(
            Diagnostic(
                code="FB_EMPTY_PDF",
                level="error",
                message="Renderer returned an empty PDF buffer.",
            )
        )

    page_count: int | None = None
    try:
        import fitz  # type: ignore

        doc = fitz.open(stream=pdf_bytes, filetype="pdf")
        try:
            page_count = doc.page_count
        finally:
            doc.close()
    except Exception as exc:
        diagnostics.append(
            Diagnostic(
                code="FB_PAGECOUNT_UNAVAILABLE",
                level="warn",
                message="Could not compute page_count in validation.",
                data={"exception": repr(exc)},
            )
        )

    checks["page_count"] = page_count
    if expected_page_count is not None and page_count is not None:
        checks["expected_page_count"] = expected_page_count
        if page_count != expected_page_count:
            diagnostics.append(
                Diagnostic(
                    code="FB_PAGECOUNT_MISMATCH",
                    level="warn",
                    message="Rendered page count differs from expected.",
                    data={"expected": expected_page_count, "actual": page_count},
                )
            )

    missing_glyphs = list(glyph_report or [])
    checks["missing_glyph_count"] = len(missing_glyphs)
    if missing_glyphs:
        diagnostics.append(
            Diagnostic(
                code="FB_MISSING_GLYPHS",
                level="warn",
                message="Glyph report contains missing glyph entries.",
                data={"sample": missing_glyphs[:10]},
            )
        )

    ok = all(item.level != "error" for item in diagnostics)
    return RenderValidation(
        ok=ok,
        bytes_written=byte_len,
        page_count=page_count,
        diagnostics=diagnostics,
        checks=checks,
    )
