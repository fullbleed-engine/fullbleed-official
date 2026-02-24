from __future__ import annotations

import json
from pathlib import Path

import fullbleed

from fullbleed.ui import LayoutGrid, el, validate_component_mount
from fullbleed.ui.accessibility import (
    A11yContract,
    A11yAttrs,
    Alert,
    Decorative,
    Details,
    ErrorText,
    FieldSet,
    FigCaption,
    Figure,
    HelpText,
    Label,
    Legend,
    LiveRegion,
    Region,
    Section,
    SignatureBlock,
    SrText,
    Status,
    Summary,
)
from fullbleed.ui.core import Document


ROOT = Path(__file__).resolve().parent
OUTPUT_DIR = ROOT / "output"
PDF_PATH = OUTPUT_DIR / "signature_accessibility_canary.pdf"
A11Y_VALIDATION_PATH = OUTPUT_DIR / "signature_accessibility_canary_a11y_validation.json"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "signature_accessibility_canary_component_mount_validation.json"
RUN_REPORT_PATH = OUTPUT_DIR / "signature_accessibility_canary_run_report.json"
PREVIEW_PNG_STEM = "signature_accessibility_canary"

CSS = """
@page { size: letter; margin: 0.5in; }
body { margin: 0; font-family: Helvetica; font-size: 11pt; color: #1a2433; }
.ui-section { margin-bottom: 0.18in; }
.ui-heading { margin: 0 0 0.08in; font-weight: 700; }
.ui-region { border: 1px solid #d8e1ee; border-radius: 4px; padding: 8px; margin-bottom: 0.12in; }
.ui-status { background: #eef8ef; border: 1px solid #b9dfbc; padding: 6px; }
.ui-alert { background: #fff4e8; border: 1px solid #f0ca9b; padding: 6px; }
.ui-live-region { background: #eef5ff; border: 1px solid #c5d9f7; padding: 6px; }
.ui-fieldset { border: 1px solid #cfd9e8; padding: 8px; }
.ui-legend { font-weight: 700; }
.ui-help-text { color: #4b627f; margin: 4px 0; }
.ui-error-text { color: #97302f; margin: 4px 0; }
.ui-signature-block { border: 1px solid #d6e0ee; border-radius: 4px; padding: 10px; }
.ui-signature-mark { display: block; margin-top: 8px; width: 2.5in; height: 0.8in; }
.ui-figure { margin: 8px 0 0; }
.ui-figcaption { font-size: 10pt; color: #40546f; }
""".strip()


def create_engine() -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        pdf_profile="tagged",
        document_lang="en-US",
        document_title="Signature Accessibility Canary",
    )


def _emit_preview_png(engine: fullbleed.PdfEngine, html: str, css: str, out_dir: Path, *, stem: str) -> list[str]:
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
    if hasattr(engine, "render_image_pages"):
        page_images = list(engine.render_image_pages(html, css, 144) or [])
        paths: list[str] = []
        for idx, image_bytes in enumerate(page_images, start=1):
            path = out_dir / f"{stem}_page{idx}.png"
            path.write_bytes(image_bytes)
            paths.append(str(path))
        return paths
    return []


def _signature_svg() -> object:
    return el(
        "svg",
        el(
            "path",
            d="M6 46 C 40 10, 90 76, 138 38 C 160 22, 182 18, 204 34",
            fill="none",
            stroke="#183b73",
            stroke_width="4",
            stroke_linecap="round",
        ),
        el(
            "path",
            d="M140 52 C 162 60, 190 62, 214 44",
            fill="none",
            stroke="#183b73",
            stroke_width="3",
            stroke_linecap="round",
        ),
        viewBox="0 0 240 80",
        width="240",
        height="80",
    )


@Document(
    page="LETTER",
    margin="0.5in",
    title="Signature Accessibility Canary",
    bootstrap=False,
)
def App(_props=None) -> object:
    signer_name = "Jane Doe"
    sig_help_id = "signature-help"
    sig_error_id = "signature-error"

    decorative_seal = Decorative(
        el(
            "svg",
            el("circle", cx="16", cy="16", r="14", fill="none", stroke="#c4cdd8", stroke_width="1.5"),
            el("path", d="M8 16 L13 21 L24 10", fill="none", stroke="#c4cdd8", stroke_width="2"),
            viewBox="0 0 32 32",
            width="24",
            height="24",
        )
    )

    return el(
        "div",
        Section(
            el(
                "p",
                "Demonstrates text-first signature semantics, decorative vs informative marks, and a11y validation hooks.",
            ),
            heading="Signature Accessibility Canary",
            heading_level=1,
        ),
        LayoutGrid(
            Region(
                Status("Signature capture completed and queued for review."),
                Alert("Audit note: timestamp source not yet notarized (example warning)."),
                LiveRegion("Background verification job pending.", live="polite"),
                label="Signature status messages",
            ),
            Region(
                FieldSet(
                    Legend("Signer review"),
                    Label("Signer record: Jane Doe"),
                    HelpText(
                        "Review the signer identity and signature method before final approval.",
                        id=sig_help_id,
                    ),
                    ErrorText("Example only: signer address proof not attached.", id=sig_error_id),
                ),
                **A11yAttrs.describedby(sig_help_id, sig_error_id),
                label="Signer review region",
            ),
            Region(
                SignatureBlock(
                    signature_status="captured",
                    signer_name=signer_name,
                    timestamp="2026-02-23T11:42:00Z",
                    signature_method="drawn_electronic",
                    reference_id="audit-42f7",
                    mark_node=_signature_svg(),
                    mark_decorative=False,
                ),
                Figure(
                    decorative_seal,
                    FigCaption("Decorative verification seal icon shown for visual trust only."),
                ),
                SrText("Assistive note: decorative verification seal conveys no additional meaning."),
                label="Signature evidence region",
            ),
            Region(
                Details(
                    Summary("Capture method details"),
                    el(
                        "p",
                        "The signature was captured electronically using a stylus and associated with the authenticated operator session.",
                    ),
                ),
                label="Signature capture details",
            ),
        ),
        class_name="signature-accessibility-canary-root",
    )


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    engine = create_engine()
    artifact = App()
    a11y_report = A11yContract().validate(artifact, mode=None)
    A11Y_VALIDATION_PATH.write_text(json.dumps(a11y_report, indent=2), encoding="utf-8")

    html = artifact.to_html(a11y_mode="raise")
    component_validation = validate_component_mount(
        engine=engine,
        node_or_component=App,
        css=CSS,
        fail_on_overflow=True,
        fail_on_css_warnings=False,
        fail_on_known_loss=False,
        fail_on_html_asset_warning=True,
    )
    COMPONENT_VALIDATION_PATH.write_text(
        json.dumps(component_validation, indent=2),
        encoding="utf-8",
    )

    bytes_written = engine.render_pdf_to_file(html, CSS, str(PDF_PATH))
    png_paths = _emit_preview_png(engine, html, CSS, OUTPUT_DIR, stem=PREVIEW_PNG_STEM)

    run_report = {
        "schema": "fullbleed.signature_a11y_canary.run.v1",
        "ok": bool(a11y_report.get("ok", False)) and bool(component_validation.get("ok", False)),
        "pdf_path": str(PDF_PATH),
        "pdf_bytes": int(bytes_written),
        "png_paths": png_paths,
        "a11y_validation_path": str(A11Y_VALIDATION_PATH),
        "component_validation_path": str(COMPONENT_VALIDATION_PATH),
    }
    RUN_REPORT_PATH.write_text(json.dumps(run_report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] A11y validation: {A11Y_VALIDATION_PATH} (ok={a11y_report.get('ok')})")
    print(
        f"[ok] Component validation: {COMPONENT_VALIDATION_PATH} (ok={component_validation.get('ok')})"
    )
    if png_paths:
        print(f"[ok] Preview PNGs: {len(png_paths)}")


if __name__ == "__main__":
    main()
