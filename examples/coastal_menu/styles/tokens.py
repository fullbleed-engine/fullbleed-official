from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ColorTokens:
    left_bg: str = "#00557f"
    center_bg: str = "#f5f7fb"
    right_bg: str = "#121f3f"
    text_primary: str = "#2a4566"
    text_muted: str = "#3d6187"
    accent: str = "#1fa5f1"
    divider: str = "#d8e0ec"
    white_soft: str = "#e7f4ff"


TOKENS = ColorTokens()


def render_root_vars() -> str:
    return (
        ":root {\n"
        f"  --coastal-left-bg: {TOKENS.left_bg};\n"
        f"  --coastal-center-bg: {TOKENS.center_bg};\n"
        f"  --coastal-right-bg: {TOKENS.right_bg};\n"
        f"  --coastal-text-primary: {TOKENS.text_primary};\n"
        f"  --coastal-text-muted: {TOKENS.text_muted};\n"
        f"  --coastal-accent: {TOKENS.accent};\n"
        f"  --coastal-divider: {TOKENS.divider};\n"
        f"  --coastal-white-soft: {TOKENS.white_soft};\n"
        "}\n"
    )
