from __future__ import annotations

from typing import Any

from .types import AccessibilityRunResult


def render_accessible_document(
    *,
    engine,
    body_html: str,
    css_text: str,
    out_dir: str,
    stem: str,
    profile: str = "cav",
    **kwargs: Any,
) -> AccessibilityRunResult:
    return engine.render_bundle(
        body_html=body_html,
        css_text=css_text,
        out_dir=out_dir,
        stem=stem,
        profile=profile,
        **kwargs,
    )


def render_document_artifact_bundle(
    *,
    engine,
    artifact,
    css_text: str,
    out_dir: str,
    stem: str,
    a11y_mode: str | None = "raise",
    profile: str = "cav",
    **kwargs: Any,
) -> AccessibilityRunResult:
    html = artifact.to_html(a11y_mode=a11y_mode) if hasattr(artifact, "to_html") else str(artifact)
    return engine.render_bundle(
        body_html=html,
        css_text=css_text,
        out_dir=out_dir,
        stem=stem,
        profile=profile,
        a11y_mode=a11y_mode,
        **kwargs,
    )
