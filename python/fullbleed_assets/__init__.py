"""Bundled assets for fullbleed.

This package contains built-in assets like Bootstrap CSS and Noto Sans font
that ship with fullbleed for quick start projects.

License notices for bundled third-party assets:
- repository: THIRD_PARTY_LICENSES.md
- package path: fullbleed_assets assets listed by `list_assets()`
"""
import os
from pathlib import Path

_PACKAGE_DIR = Path(__file__).parent


def asset_path(name: str) -> str:
    """Get the absolute path to a bundled asset file.
    
    Args:
        name: Relative path within the assets package, e.g. "bootstrap.min.css"
              or "fonts/NotoSans-Regular.ttf"
    
    Returns:
        Absolute path to the asset file.
    
    Raises:
        FileNotFoundError: If the asset doesn't exist.
    """
    path = _PACKAGE_DIR / name
    if not path.exists():
        raise FileNotFoundError(f"Bundled asset not found: {name}")
    return str(path)


def list_assets() -> list[str]:
    """List all bundled asset files."""
    assets = []
    for root, dirs, files in os.walk(_PACKAGE_DIR):
        for f in files:
            if not f.startswith("__"):
                rel = Path(root).relative_to(_PACKAGE_DIR) / f
                assets.append(str(rel))
    return assets
