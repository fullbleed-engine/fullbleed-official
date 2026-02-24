from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

import fullbleed
from fullbleed.accessibility import AccessibilityEngine

from fullbleed.ui import LayoutGrid, Text, el, validate_component_mount
from fullbleed.ui.accessibility import (
    A11yContract,
    Alert,
    ColumnHeader,
    DataCell,
    Decorative,
    FigCaption,
    Figure,
    FieldGrid,
    FieldItem,
    Region,
    RowHeader,
    Section,
    SemanticTable,
    SemanticTableBody,
    SemanticTableHead,
    SemanticTableRow,
    SignatureBlock,
    Status,
)
from fullbleed.ui.core import Document


ROOT = Path(__file__).resolve().parent
OUTPUT_DIR = ROOT / "output"
CSS_PATH = ROOT / "styles" / "report.css"

HTML_PATH = OUTPUT_DIR / "accessibility_scaffold.html"
CSS_ARTIFACT_PATH = OUTPUT_DIR / "accessibility_scaffold.css"
PDF_PATH = OUTPUT_DIR / "accessibility_scaffold.pdf"
A11Y_VALIDATION_PATH = OUTPUT_DIR / "accessibility_scaffold_a11y_validation.json"
COMPONENT_VALIDATION_PATH = OUTPUT_DIR / "accessibility_scaffold_component_mount_validation.json"
RUN_REPORT_PATH = OUTPUT_DIR / "accessibility_scaffold_run_report.json"
ENGINE_A11Y_VERIFY_PATH = OUTPUT_DIR / "accessibility_scaffold_a11y_verify_engine.json"
ENGINE_PMR_PATH = OUTPUT_DIR / "accessibility_scaffold_pmr_engine.json"
CLAIM_EVIDENCE_PATH = OUTPUT_DIR / "accessibility_scaffold_claim_evidence.json"
PREVIEW_PNG_STEM = "accessibility_scaffold"


@dataclass(frozen=True)
class Transaction:
    date: str
    description: str
    amount: str
    balance: str


TRANSACTIONS: tuple[Transaction, ...] = (
    Transaction("2026-02-10", "Invoice payment", "$500.00", "$1,240.00"),
    Transaction("2026-02-14", "Service charge", "-$12.00", "$1,228.00"),
    Transaction("2026-02-20", "Adjustment", "$40.00", "$1,268.00"),
)


def create_engine() -> AccessibilityEngine:
    return AccessibilityEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        document_lang="en-US",
        document_title="Accessibility Scaffold",
        strict=False,
    )


def _emit_preview_png(engine: fullbleed.PdfEngine, html: str, css: str, out_dir: Path, *, stem: str) -> list[str]:
    if hasattr(engine, "render_image_pages_to_dir"):
        return list(engine.render_image_pages_to_dir(html, css, str(out_dir), 144, stem) or [])
    if hasattr(engine, "render_image_pages"):
        page_images = list(engine.render_image_pages(html, css, 144) or [])
        out_paths: list[str] = []
        for idx, image_bytes in enumerate(page_images, start=1):
            path = out_dir / f"{stem}_page{idx}.png"
            path.write_bytes(image_bytes)
            out_paths.append(str(path))
        return out_paths
    return []


def _emit_engine_audit_reports(
    *,
    engine: fullbleed.PdfEngine,
    html_path: Path,
    css_path: Path,
    png_paths: list[str],
    a11y_report: dict,
    component_validation: dict,
    claim_evidence: dict | None,
) -> dict:
    out = {
        "engine_a11y_verify_path": None,
        "engine_pmr_path": None,
        "engine_a11y_verify_ok": None,
        "engine_pmr_ok": None,
        "engine_pmr_score": None,
        "engine_a11y_contrast_seed_verdict": None,
        "engine_a11y_natural_pass_ok": None,
        "engine_a11y_natural_nonpass_rule_ids": [],
        "warnings": [],
    }
    if not hasattr(engine, "verify_accessibility_artifacts"):
        out["warnings"].append("PdfEngine.verify_accessibility_artifacts is unavailable.")
        return out
    if not hasattr(engine, "verify_paged_media_rank_artifacts"):
        out["warnings"].append("PdfEngine.verify_paged_media_rank_artifacts is unavailable.")
        return out

    contrast_png = png_paths[0] if png_paths else None
    try:
        a11y_verify = engine.verify_accessibility_artifacts(
            str(html_path),
            str(css_path),
            profile="strict",
            mode="error",
            render_preview_png_path=contrast_png,
            a11y_report=a11y_report,
            claim_evidence=claim_evidence,
        )
    except TypeError:
        try:
            a11y_verify = engine.verify_accessibility_artifacts(
                str(html_path),
                str(css_path),
                profile="strict",
                mode="error",
                render_preview_png_path=contrast_png,
                a11y_report=a11y_report,
            )
            out["warnings"].append(
                "Engine verifier does not accept claim_evidence; claim seed rules may remain manual_needed."
            )
        except TypeError:
            try:
                a11y_verify = engine.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile="strict",
                    mode="error",
                    render_preview_png_path=contrast_png,
                )
                out["warnings"].append(
                    "Engine verifier does not accept a11y_report or claim_evidence; pre-render bridge correlation not evaluated and claim seed rules may remain manual_needed."
                )
            except TypeError:
                a11y_verify = engine.verify_accessibility_artifacts(
                    str(html_path),
                    str(css_path),
                    profile="strict",
                    mode="error",
                )
                out["warnings"].append(
                    "Engine verifier does not accept render_preview_png_path, a11y_report, or claim_evidence; contrast seed, pre-render bridge correlation, and claim seed evidence were not evaluated."
                )
    ENGINE_A11Y_VERIFY_PATH.write_text(json.dumps(a11y_verify, indent=2), encoding="utf-8")
    out["engine_a11y_verify_path"] = str(ENGINE_A11Y_VERIFY_PATH)
    out["engine_a11y_verify_ok"] = bool((a11y_verify.get("gate") or {}).get("ok", False))
    nonpass_rule_ids = []
    for finding in a11y_verify.get("findings") or []:
        verdict = str(finding.get("verdict") or "")
        rule_id = str(finding.get("rule_id") or "")
        if verdict in {"fail", "warn", "manual_needed"} and rule_id != "fb.a11y.claim.wcag20aa_level_readiness":
            if rule_id not in nonpass_rule_ids:
                nonpass_rule_ids.append(rule_id)
    out["engine_a11y_natural_nonpass_rule_ids"] = nonpass_rule_ids
    out["engine_a11y_natural_pass_ok"] = len(nonpass_rule_ids) == 0
    contrast_rows = [
        f
        for f in (a11y_verify.get("findings") or [])
        if f.get("rule_id") == "fb.a11y.contrast.minimum_render_seed"
    ]
    if contrast_rows:
        out["engine_a11y_contrast_seed_verdict"] = contrast_rows[0].get("verdict")

    pmr = engine.verify_paged_media_rank_artifacts(
        str(html_path),
        str(css_path),
        profile="strict",
        mode="error",
        overflow_count=int(component_validation.get("overflow_count") or 0),
        known_loss_count=int(component_validation.get("known_loss_count") or 0),
        render_page_count=len(png_paths),
    )
    ENGINE_PMR_PATH.write_text(json.dumps(pmr, indent=2), encoding="utf-8")
    out["engine_pmr_path"] = str(ENGINE_PMR_PATH)
    out["engine_pmr_ok"] = bool((pmr.get("gate") or {}).get("ok", False))
    rank = pmr.get("rank") or {}
    if "score" in rank:
        out["engine_pmr_score"] = rank.get("score")
    return out


def _signature_svg() -> object:
    return el(
        "svg",
        el(
            "path",
            d="M6 46 C 40 12, 88 76, 136 38 C 160 20, 184 20, 208 34",
            fill="none",
            stroke="#183b73",
            stroke_width="4",
            stroke_linecap="round",
        ),
        el(
            "path",
            d="M142 50 C 164 58, 192 60, 214 44",
            fill="none",
            stroke="#183b73",
            stroke_width="3",
            stroke_linecap="round",
        ),
        viewBox="0 0 220 80",
        width="220",
        height="80",
    )


def _verification_seal_svg() -> object:
    return el(
        "svg",
        el("circle", cx="16", cy="16", r="14", fill="none", stroke="#c4cdd8", stroke_width="1.5"),
        el("path", d="M8 16 L13 21 L24 10", fill="none", stroke="#c4cdd8", stroke_width="2"),
        viewBox="0 0 32 32",
        width="24",
        height="24",
    )


@Document(
    page="LETTER",
    margin="0.5in",
    title="Accessibility Scaffold",
    bootstrap=False,
)
def App(_props=None) -> object:
    summary_heading_id = "summary-heading"
    transactions_heading_id = "transactions-heading"
    signature_heading_id = "signature-heading"

    return el(
        "div",
        Section(
            el(
                "p",
                "Accessibility-first starter using semantic fields, semantic tables, labeled regions, and text-first signature semantics.",
            ),
            heading="Accessibility Scaffold",
            heading_level=1,
        ),
        LayoutGrid(
            Region(
                Text("Account Summary", tag="h2", id=summary_heading_id),
                FieldGrid(
                    FieldItem("Account", "A-1042"),
                    FieldItem("Statement period", "2026-02-01 to 2026-02-29"),
                    FieldItem("Owner", "Jane Doe"),
                ),
                labelledby=summary_heading_id,
            ),
            Region(
                Text("Transactions", tag="h2", id=transactions_heading_id),
                SemanticTable(
                    SemanticTableHead(
                        SemanticTableRow(
                            ColumnHeader("Date"),
                            ColumnHeader("Description"),
                            ColumnHeader("Amount"),
                            ColumnHeader("Balance"),
                        )
                    ),
                    SemanticTableBody(
                        *[
                            SemanticTableRow(
                                RowHeader(tx.date),
                                DataCell(tx.description),
                                DataCell(tx.amount),
                                DataCell(tx.balance),
                            )
                            for tx in TRANSACTIONS
                        ]
                    ),
                    caption="Transaction table",
                ),
                labelledby=transactions_heading_id,
            ),
            Region(
                Text("Signature Evidence", tag="h2", id=signature_heading_id),
                Status("Signature capture completed and recorded."),
                Alert("Example alert: audit verification pending final review."),
                SignatureBlock(
                    signature_status="captured",
                    signer_name="Jane Doe",
                    timestamp="2026-02-23T11:42:00Z",
                    signature_method="drawn_electronic",
                    reference_id="audit-42f7",
                    mark_node=_signature_svg(),
                    mark_decorative=False,
                ),
                Figure(
                    Decorative(_verification_seal_svg()),
                    FigCaption("Decorative verification seal shown for visual trust only."),
                ),
                labelledby=signature_heading_id,
            ),
        ),
        class_name="scaffold-root",
    )


def _build_claim_evidence_payload() -> dict:
    return {
        "schema": "fullbleed.a11y.claim_evidence.v1",
        "delivery_target": "html",
        "document_use_case": "scaffold",
        "technology_support": {
            "assessed": True,
            "basis_recorded": True,
            "relied_upon_technologies": ["html", "css"],
            "active_content_present": False,
            "assessment_basis": "Starter scaffold renders static HTML/CSS document content only.",
        },
        "section508": {
            "scope_declared": True,
            "profile": "section508.revised.e205.html",
            "public_facing_determination_recorded": True,
            "public_facing_content": False,
            "official_communications_determination_recorded": True,
            "official_communications": False,
            "nara_exception_determination_recorded": True,
            "nara_exception_applies": False,
            "determination_basis": "Starter scaffold records applicability determinations explicitly for audit traceability.",
        },
        "wcag20": {
            "keyboard_assessed": True,
            "keyboard_basis_recorded": True,
            "keyboard_trap_assessed": True,
            "keyboard_trap_basis_recorded": True,
            "on_input_assessed": True,
            "on_input_basis_recorded": True,
            "on_focus_assessed": True,
            "on_focus_basis_recorded": True,
            "timing_adjustable_scope_declared": False,
            "timing_adjustable_assessed": True,
            "timing_adjustable_basis_recorded": True,
            "pause_stop_hide_scope_declared": False,
            "pause_stop_hide_assessed": True,
            "pause_stop_hide_basis_recorded": True,
            "three_flashes_scope_declared": False,
            "three_flashes_assessed": True,
            "three_flashes_basis_recorded": True,
            "audio_control_scope_declared": False,
            "audio_control_assessed": True,
            "audio_control_basis_recorded": True,
            "use_of_color_scope_declared": False,
            "use_of_color_assessed": True,
            "use_of_color_basis_recorded": True,
            "resize_text_scope_declared": False,
            "resize_text_assessed": True,
            "resize_text_basis_recorded": True,
            "images_of_text_scope_declared": False,
            "images_of_text_assessed": True,
            "images_of_text_basis_recorded": True,
            "prerecorded_av_alternative_scope_declared": False,
            "prerecorded_av_alternative_assessed": True,
            "prerecorded_av_alternative_basis_recorded": True,
            "prerecorded_captions_scope_declared": False,
            "prerecorded_captions_assessed": True,
            "prerecorded_captions_basis_recorded": True,
            "prerecorded_audio_description_or_media_alternative_scope_declared": False,
            "prerecorded_audio_description_or_media_alternative_assessed": True,
            "prerecorded_audio_description_or_media_alternative_basis_recorded": True,
            "live_captions_scope_declared": False,
            "live_captions_assessed": True,
            "live_captions_basis_recorded": True,
            "prerecorded_audio_description_scope_declared": False,
            "prerecorded_audio_description_assessed": True,
            "prerecorded_audio_description_basis_recorded": True,
            "meaningful_sequence_scope_declared": True,
            "meaningful_sequence_assessed": True,
            "meaningful_sequence_basis_recorded": True,
            "error_suggestion_scope_declared": False,
            "error_suggestion_assessed": True,
            "error_suggestion_basis_recorded": True,
            "error_prevention_scope_declared": False,
            "error_prevention_assessed": True,
            "error_prevention_basis_recorded": True,
            "consistent_identification_assessed": True,
            "consistent_identification_basis_recorded": True,
            "multiple_ways_scope_declared": False,
            "multiple_ways_assessed": True,
            "multiple_ways_basis_recorded": True,
            "consistent_navigation_scope_declared": False,
            "consistent_navigation_assessed": True,
            "consistent_navigation_basis_recorded": True,
            "assessment_basis": "Starter scaffold records a manual consistency-identification review for repeated interactive component labeling.",
        },
    }


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    css = CSS_PATH.read_text(encoding="utf-8")

    engine = create_engine()
    artifact = App()

    a11y_report = A11yContract().validate(artifact, mode=None)
    A11Y_VALIDATION_PATH.write_text(json.dumps(a11y_report, indent=2), encoding="utf-8")
    claim_evidence = _build_claim_evidence_payload()
    CLAIM_EVIDENCE_PATH.write_text(json.dumps(claim_evidence, indent=2), encoding="utf-8")

    # Validate the authored tree first; bundle emission/renders/audits then run through
    # the accessibility runtime surface so PDF output is explicitly PDF/UA-targeted.
    artifact.to_html(a11y_mode="raise")

    component_validation = validate_component_mount(
        engine=engine,
        node_or_component=App,
        css=css,
        fail_on_overflow=True,
        fail_on_css_warnings=False,
        fail_on_known_loss=False,
        fail_on_html_asset_warning=True,
    )
    COMPONENT_VALIDATION_PATH.write_text(
        json.dumps(component_validation, indent=2),
        encoding="utf-8",
    )

    bundle = engine.render_bundle(
        body_html=artifact.root.to_html(),
        css_text=css,
        out_dir=str(OUTPUT_DIR),
        stem=PREVIEW_PNG_STEM,
        profile="strict",
        a11y_mode="raise",
        a11y_report=a11y_report,
        claim_evidence=claim_evidence,
        component_validation=component_validation,
        render_preview_png=True,
        run_verifier=True,
        run_pmr=True,
        run_pdf_ua_seed_verify=True,
        emit_reading_order_trace=True,
        emit_pdf_structure_trace=True,
    )
    bundle_run = bundle.run_report or {}
    bytes_written = int((bundle_run.get("metrics") or {}).get("pdf_bytes") or 0)
    png_paths = list(bundle_run.get("render_preview_png_paths") or [])
    engine_audits = {
        "engine_a11y_verify_path": bundle.paths.get("engine_a11y_verify_path"),
        "engine_pmr_path": bundle.paths.get("engine_pmr_path"),
        "engine_a11y_verify_ok": bundle_run.get("engine_a11y_verify_ok"),
        "engine_pmr_ok": bundle_run.get("engine_pmr_ok"),
        "engine_pmr_score": bundle_run.get("engine_pmr_score"),
        "engine_a11y_contrast_seed_verdict": None,
        "engine_a11y_natural_pass_ok": None,
        "engine_a11y_natural_nonpass_rule_ids": [],
        "pdf_ua_seed_verify_path": bundle.paths.get("pdf_ua_seed_verify_path"),
        "reading_order_trace_path": bundle.paths.get("reading_order_trace_path"),
        "reading_order_trace_render_path": bundle.paths.get("reading_order_trace_render_path"),
        "pdf_structure_trace_path": bundle.paths.get("pdf_structure_trace_path"),
        "pdf_structure_trace_render_path": bundle.paths.get("pdf_structure_trace_render_path"),
        "reading_order_trace_cross_check": bundle_run.get("reading_order_trace_cross_check"),
        "pdf_structure_trace_cross_check": bundle_run.get("pdf_structure_trace_cross_check"),
        "pdf_ua_seed_ok": bundle_run.get("pdf_ua_seed_ok"),
        "warnings": list(bundle.warnings or []),
    }
    if bundle.verifier_report:
        nonpass_rule_ids = []
        for finding in bundle.verifier_report.get("findings") or []:
            verdict = str(finding.get("verdict") or "")
            rule_id = str(finding.get("rule_id") or "")
            if verdict in {"fail", "warn", "manual_needed"} and rule_id != "fb.a11y.claim.wcag20aa_level_readiness":
                if rule_id not in nonpass_rule_ids:
                    nonpass_rule_ids.append(rule_id)
        engine_audits["engine_a11y_natural_nonpass_rule_ids"] = nonpass_rule_ids
        engine_audits["engine_a11y_natural_pass_ok"] = len(nonpass_rule_ids) == 0
        contrast_rows = [
            f
            for f in (bundle.verifier_report.get("findings") or [])
            if f.get("rule_id") == "fb.a11y.contrast.minimum_render_seed"
        ]
        if contrast_rows:
            engine_audits["engine_a11y_contrast_seed_verdict"] = contrast_rows[0].get("verdict")

    run_report = {
        "schema": "fullbleed.new_accessible_scaffold.run.v1",
        "ok": bool(a11y_report.get("ok", False)) and bool(component_validation.get("ok", False)),
        "html_path": str(HTML_PATH),
        "css_path": str(CSS_ARTIFACT_PATH),
        "css_source_path": str(CSS_PATH),
        "pdf_path": str(PDF_PATH),
        "pdf_bytes": bytes_written,
        "png_paths": png_paths,
        "a11y_validation_path": str(A11Y_VALIDATION_PATH),
        "component_validation_path": str(COMPONENT_VALIDATION_PATH),
        "claim_evidence_path": str(CLAIM_EVIDENCE_PATH),
        "engine_a11y_verify_path": engine_audits.get("engine_a11y_verify_path"),
        "engine_pmr_path": engine_audits.get("engine_pmr_path"),
        "pdf_ua_seed_verify_path": engine_audits.get("pdf_ua_seed_verify_path"),
        "reading_order_trace_path": engine_audits.get("reading_order_trace_path"),
        "reading_order_trace_render_path": engine_audits.get("reading_order_trace_render_path"),
        "pdf_structure_trace_path": engine_audits.get("pdf_structure_trace_path"),
        "pdf_structure_trace_render_path": engine_audits.get("pdf_structure_trace_render_path"),
        "reading_order_trace_cross_check": engine_audits.get("reading_order_trace_cross_check"),
        "pdf_structure_trace_cross_check": engine_audits.get("pdf_structure_trace_cross_check"),
        "engine_audits": engine_audits,
    }
    RUN_REPORT_PATH.write_text(json.dumps(run_report, indent=2), encoding="utf-8")

    print(f"[ok] Wrote {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] A11y validation: {A11Y_VALIDATION_PATH} (ok={a11y_report.get('ok')})")
    print(
        f"[ok] Component validation: {COMPONENT_VALIDATION_PATH} (ok={component_validation.get('ok')})"
    )
    if engine_audits.get("engine_a11y_verify_path"):
        print(
            f"[ok] Engine a11y verify: {engine_audits['engine_a11y_verify_path']} "
            f"(ok={engine_audits.get('engine_a11y_verify_ok')})"
        )
    if engine_audits.get("engine_pmr_path"):
        print(
            f"[ok] Engine PMR: {engine_audits['engine_pmr_path']} "
            f"(ok={engine_audits.get('engine_pmr_ok')}, score={engine_audits.get('engine_pmr_score')})"
        )
    for warning in engine_audits.get("warnings", []):
        print(f"[warn] {warning}")
    if png_paths:
        print(f"[ok] Preview PNGs: {len(png_paths)}")


if __name__ == "__main__":
    main()
