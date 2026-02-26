from __future__ import annotations

import json
from pathlib import Path

import fullbleed


ROOT = Path(__file__).resolve().parent
TEMPLATE_HTML_PATH = ROOT / "templates" / "statement.html"
TEMPLATE_CSS_PATH = ROOT / "templates" / "statement.css"
OUTPUT_DIR = ROOT / "output"
HTML_PATH = OUTPUT_DIR / "statement.html"
CSS_PATH = OUTPUT_DIR / "statement.css"
PDF_PATH = OUTPUT_DIR / "statement.pdf"
RUN_REPORT_PATH = OUTPUT_DIR / "statement_run_report.json"


def create_engine() -> fullbleed.PdfEngine:
    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0.5in",
        document_lang="en",
        document_title="Account Statement",
    )
    if hasattr(engine, "document_css_href"):
        engine.document_css_href = CSS_PATH.name
    if hasattr(engine, "document_css_source_path"):
        engine.document_css_source_path = str(TEMPLATE_CSS_PATH)
    if hasattr(engine, "document_css_media"):
        engine.document_css_media = "all"
    if hasattr(engine, "document_css_required"):
        engine.document_css_required = True
    return engine


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    html = TEMPLATE_HTML_PATH.read_text(encoding="utf-8")
    css = TEMPLATE_CSS_PATH.read_text(encoding="utf-8")

    engine = create_engine()
    html_css_emit_status = "ok"
    emitted: dict[str, object] = {}
    if hasattr(engine, "emit_artifacts"):
        emitted = dict(
            engine.emit_artifacts(
                html,
                css,
                str(HTML_PATH),
                str(CSS_PATH),
                False,
            )
        )
    else:
        html_css_emit_status = "fallback (native emitter unavailable)"
        HTML_PATH.write_text(html, encoding="utf-8")
        CSS_PATH.write_text(css, encoding="utf-8")

    pdf_bytes = int(engine.render_pdf_to_file(html, css, str(PDF_PATH)))
    metadata = {}
    if hasattr(engine, "document_metadata"):
        metadata = dict(engine.document_metadata())

    run_report = {
        "schema": "fullbleed.new_template.statement.run.v1",
        "ok": True,
        "html_path": str(HTML_PATH),
        "css_path": str(CSS_PATH),
        "css_source_path": str(TEMPLATE_CSS_PATH),
        "pdf_path": str(PDF_PATH),
        "pdf_bytes": pdf_bytes,
        "html_css_emit_status": html_css_emit_status,
        "document_css_href": metadata.get("document_css_href"),
        "document_css_source_path": metadata.get("document_css_source_path"),
        "document_css_media": metadata.get("document_css_media"),
        "document_css_required": metadata.get("document_css_required"),
        "css_link_href": emitted.get("css_link_href"),
        "css_link_media": emitted.get("css_link_media"),
        "css_link_injected": emitted.get("css_link_injected"),
        "css_link_preexisting": emitted.get("css_link_preexisting"),
    }
    RUN_REPORT_PATH.write_text(json.dumps(run_report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote {PDF_PATH} ({pdf_bytes} bytes)")
    print(f"[ok] HTML artifact: {HTML_PATH} ({html_css_emit_status})")
    print(f"[ok] CSS artifact: {CSS_PATH} ({html_css_emit_status})")
    print(f"[ok] Run report: {RUN_REPORT_PATH}")


if __name__ == "__main__":
    main()
