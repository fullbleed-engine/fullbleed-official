# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from pathlib import Path
from typing import Dict, List, Optional, Any
try:
    import tomllib
except ImportError:
    import tomli as tomllib
import sys

# Default configuration structure
DEFAULT_CONFIG = {
    "project": {
        "name": "fullbleed-project",
        "version": "0.1.0",
        "entrypoint": "report.py:create_engine",
    },
    "build": {
        "output": "dist/report.pdf",
        "html": "templates/index.html",
        "css": [], # List of CSS files
    },
    "assets": {
        # "package-name": { "version": "...", "kind": "..." }
    },
    "engine": {
        # Engine options override
    }
}

class Config:
    def __init__(self, data: Dict[str, Any], path: Path):
        self.data = data
        self.path = path
        self.root = path.parent

    @classmethod
    def load(cls, path: Optional[Path] = None) -> "Config":
        """Load configuration from fullbleed.toml."""
        if path is None:
            # Look in current directory and parents
            cwd = Path.cwd()
            path = cwd / "fullbleed.toml"
            if not path.exists():
                # Fallback to defaults? No, error is better for dev workflow
                raise FileNotFoundError(f"No fullbleed.toml found at {path}. Run 'fullbleed init' to create one.")

        try:
            with open(path, "rb") as f:
                data = tomllib.load(f)
        except Exception as e:
            raise ValueError(f"Failed to parse {path}: {e}")
            
        return cls(data, path)

    @property
    def project(self) -> Dict[str, Any]:
        return self.data.get("project", {})

    @property
    def build(self) -> Dict[str, Any]:
        return self.data.get("build", {})

    @property
    def assets(self) -> Dict[str, Any]:
        return self.data.get("assets", {})
    
    @property
    def engine(self) -> Dict[str, Any]:
        return self.data.get("engine", {})

    def resolve_path(self, relative_path: str) -> Path:
        return self.root / relative_path

    # Helpers for common fields
    def get_output_path(self) -> Path:
        out = self.build.get("output", "dist/report.pdf")
        return self.resolve_path(out)

    def get_html_path(self) -> Optional[Path]:
        html = self.build.get("html")
        return self.resolve_path(html) if html else None
    
    def get_css_paths(self) -> List[Path]:
        css = self.build.get("css", [])
        if isinstance(css, str):
            css = [css]
        return [self.resolve_path(c) for c in css]

    def get_entrypoint(self) -> str:
        return self.project.get("entrypoint", "report.py:create_engine")
