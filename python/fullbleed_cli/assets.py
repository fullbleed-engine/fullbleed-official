# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Asset pipeline commands for fullbleed.

Provides commands for installing, listing, verifying, and locking asset packages.
"""
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Dict, List, Optional

from .cache import get_cache_dir, ensure_cache_dir, get_package_cache_path


# Built-in assets that ship with fullbleed_assets package
BUILTIN_ASSETS = {
    "bootstrap": {
        "versions": ["5.0.0"],
        "default": "5.0.0",
        "kind": "css",
        "description": "Bootstrap CSS framework",
        "license": "MIT",
        "license_url": "https://raw.githubusercontent.com/twbs/bootstrap/v5.0.0/LICENSE",
    },
    "bootstrap-icons": {
        "versions": ["1.11.3"],
        "default": "1.11.3",
        "kind": "icon",
        "description": "Bootstrap Icons SVG sprite",
        "license": "MIT",
        "license_url": "https://raw.githubusercontent.com/twbs/icons/v1.11.3/LICENSE",
    },
    "noto-sans": {
        "versions": ["regular"],
        "default": "regular",
        "kind": "font",
        "description": "Noto Sans font family (Regular weight)",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt",
    },
}

# Remote assets that can be downloaded from the internet
# Using Google Fonts GitHub repo as the source
# https://github.com/google/fonts/tree/main/ofl/notosans
REMOTE_ASSETS = {
    "noto-sans-regular": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt",
        "description": "Noto Sans Regular (400)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSans-Variable.ttf",
        "sha256": None
    },
    "noto-sans-italic": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt",
        "description": "Noto Sans Italic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans-Italic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSans-Italic-Variable.ttf",
        "sha256": None
    },
    "noto-serif-regular": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt",
        "description": "Noto Serif Regular (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSerif-Variable.ttf",
        "sha256": None
    },
    "noto-serif-italic": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt",
        "description": "Noto Serif Italic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif-Italic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSerif-Italic-Variable.ttf",
        "sha256": None
    },
    "noto-sans-mono": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/OFL.txt",
        "description": "Noto Sans Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/NotoSansMono%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansMono-Variable.ttf",
        "sha256": None
    },
    "noto-sans-jp": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/OFL.txt",
        "description": "Noto Sans Japanese (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
        "filename": "NotoSansJP-Variable.ttf",
        "sha256": None
    },
    "noto-sans-sc": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/OFL.txt",
        "description": "Noto Sans Simplified Chinese (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
        "filename": "NotoSansSC-Variable.ttf",
        "sha256": None
    },
    "noto-sans-kr": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/OFL.txt",
        "description": "Noto Sans Korean (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf",
        "filename": "NotoSansKR-Variable.ttf",
        "sha256": None
    },
    "noto-sans-arabic": {
        "kind": "font",
        "version": "2.010",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/OFL.txt",
        "description": "Noto Sans Arabic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansArabic-Variable.ttf",
        "sha256": None
    },
    "noto-sans-hebrew": {
        "kind": "font",
        "version": "2.003",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/OFL.txt",
        "description": "Noto Sans Hebrew (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansHebrew-Variable.ttf",
        "sha256": None
    },
    "noto-sans-thai": {
        "kind": "font",
        "version": "2.002",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/OFL.txt",
        "description": "Noto Sans Thai (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/NotoSansThai%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansThai-Variable.ttf",
        "sha256": None
    },
    "noto-color-emoji": {
        "kind": "font",
        "version": "2.047",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/OFL.txt",
        "description": "Noto Color Emoji",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/NotoColorEmoji-Regular.ttf",
        "filename": "NotoColorEmoji-Regular.ttf",
        "sha256": None
    },
    "inter": {
        "kind": "font",
        "version": "4.0",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inter/OFL.txt",
        "description": "Inter - Modern sans-serif for UI",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf",
        "filename": "Inter-Variable.ttf",
        "sha256": None
    },
    "open-sans": {
        "kind": "font",
        "version": "wdth,wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OFL.txt",
        "description": "Open Sans (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OpenSans%5Bwdth%2Cwght%5D.ttf",
        "filename": "OpenSans-Variable.ttf",
        "sha256": None
    },
    "lato": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lato/OFL.txt",
        "description": "Lato (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lato/Lato-Regular.ttf",
        "filename": "Lato-Regular.ttf",
        "sha256": None
    },
    "montserrat": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/OFL.txt",
        "description": "Montserrat (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/Montserrat%5Bwght%5D.ttf",
        "filename": "Montserrat-Variable.ttf",
        "sha256": None
    },
    "poppins": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/OFL.txt",
        "description": "Poppins (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/Poppins-Regular.ttf",
        "filename": "Poppins-Regular.ttf",
        "sha256": None
    },
    "raleway": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/OFL.txt",
        "description": "Raleway (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/Raleway%5Bwght%5D.ttf",
        "filename": "Raleway-Variable.ttf",
        "sha256": None
    },
    "oswald": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/OFL.txt",
        "description": "Oswald (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/Oswald%5Bwght%5D.ttf",
        "filename": "Oswald-Variable.ttf",
        "sha256": None
    },
    "nunito": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/OFL.txt",
        "description": "Nunito (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/Nunito%5Bwght%5D.ttf",
        "filename": "Nunito-Variable.ttf",
        "sha256": None
    },
    "rubik": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/OFL.txt",
        "description": "Rubik (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/Rubik%5Bwght%5D.ttf",
        "filename": "Rubik-Variable.ttf",
        "sha256": None
    },
    "work-sans": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/OFL.txt",
        "description": "Work Sans (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/WorkSans%5Bwght%5D.ttf",
        "filename": "WorkSans-Variable.ttf",
        "sha256": None
    },
    "playfair-display": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/OFL.txt",
        "description": "Playfair Display (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/PlayfairDisplay%5Bwght%5D.ttf",
        "filename": "PlayfairDisplay-Variable.ttf",
        "sha256": None
    },
    "lora": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lora/OFL.txt",
        "description": "Lora (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lora/Lora%5Bwght%5D.ttf",
        "filename": "Lora-Variable.ttf",
        "sha256": None
    },
    "pt-serif": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/OFL.txt",
        "description": "PT Serif (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/PT_Serif-Web-Regular.ttf",
        "filename": "PTSerif-Regular.ttf",
        "sha256": None
    },
    "crimson-text": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/OFL.txt",
        "description": "Crimson Text (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/CrimsonText-Regular.ttf",
        "filename": "CrimsonText-Regular.ttf",
        "sha256": None
    },
    "libre-baskerville": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/OFL.txt",
        "description": "Libre Baskerville (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/LibreBaskerville%5Bwght%5D.ttf",
        "filename": "LibreBaskerville-Variable.ttf",
        "sha256": None
    },
    "arvo": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/OFL.txt",
        "description": "Arvo (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/Arvo-Regular.ttf",
        "filename": "Arvo-Regular.ttf",
        "sha256": None
    },
    "eb-garamond": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/OFL.txt",
        "description": "EB Garamond (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/EBGaramond%5Bwght%5D.ttf",
        "filename": "EBGaramond-Variable.ttf",
        "sha256": None
    },
    "roboto-mono": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/OFL.txt",
        "description": "Roboto Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/RobotoMono%5Bwght%5D.ttf",
        "filename": "RobotoMono-Variable.ttf",
        "sha256": None
    },
    "source-code-pro": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/OFL.txt",
        "description": "Source Code Pro (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/SourceCodePro%5Bwght%5D.ttf",
        "filename": "SourceCodePro-Variable.ttf",
        "sha256": None
    },
    "fira-code": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/OFL.txt",
        "description": "Fira Code (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/FiraCode%5Bwght%5D.ttf",
        "filename": "FiraCode-Variable.ttf",
        "sha256": None
    },
    "jetbrains-mono": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/OFL.txt",
        "description": "JetBrains Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf",
        "filename": "JetBrainsMono-Variable.ttf",
        "sha256": None
    },
    "libre-barcode-128": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128/OFL.txt",
        "description": "Libre Barcode 128",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128/LibreBarcode128-Regular.ttf",
        "filename": "LibreBarcode128-Regular.ttf",
        "sha256": None
    },
    "libre-barcode-128-text": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128text/OFL.txt",
        "description": "Libre Barcode 128 Text (human-readable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode128text/LibreBarcode128Text-Regular.ttf",
        "filename": "LibreBarcode128Text-Regular.ttf",
        "sha256": None
    },
    "libre-barcode-39": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39/OFL.txt",
        "description": "Libre Barcode 39",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39/LibreBarcode39-Regular.ttf",
        "filename": "LibreBarcode39-Regular.ttf",
        "sha256": None
    },
    "libre-barcode-39-text": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39text/OFL.txt",
        "description": "Libre Barcode 39 Text (human-readable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39text/LibreBarcode39Text-Regular.ttf",
        "filename": "LibreBarcode39Text-Regular.ttf",
        "sha256": None
    },
    "libre-barcode-39-extended": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39extended/OFL.txt",
        "description": "Libre Barcode 39 Extended (full ASCII subset)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcode39extended/LibreBarcode39Extended-Regular.ttf",
        "filename": "LibreBarcode39Extended-Regular.ttf",
        "sha256": None
    },
    "libre-barcode-ean13-text": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcodeean13text/OFL.txt",
        "description": "Libre Barcode EAN13 Text",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebarcodeean13text/LibreBarcodeEAN13Text-Regular.ttf",
        "filename": "LibreBarcodeEAN13Text-Regular.ttf",
        "sha256": None
    }
}


# ============================================================================
# Project Detection (cargo/npm-style CWD-relative operations)
# ============================================================================

def detect_project_root() -> Optional[Path]:
    """Find project root by looking for fullbleed project markers.
    
    Returns the project root directory if found, None otherwise.
    """
    cwd = Path.cwd()
    markers = ["assets.lock.json", "report.py", "fullbleed.toml"]
    for marker in markers:
        if (cwd / marker).exists():
            return cwd
    return None


def _download_file(url: str, dest: Path, show_progress: bool = True) -> int:
    """Download a file from URL to destination path. Returns bytes downloaded."""
    import urllib.request
    import urllib.error
    
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            total_size = int(response.headers.get('Content-Length', 0))
            downloaded = 0
            chunk_size = 8192
            
            dest.parent.mkdir(parents=True, exist_ok=True)
            with open(dest, 'wb') as f:
                while True:
                    chunk = response.read(chunk_size)
                    if not chunk:
                        break
                    f.write(chunk)
                    downloaded += len(chunk)
                    
                    if show_progress and total_size > 0:
                        pct = (downloaded / total_size) * 100
                        sys.stderr.write(f"\r  Downloading: {pct:.1f}% ({downloaded // 1024} KB)")
                        sys.stderr.flush()
            
            if show_progress and total_size > 0:
                sys.stderr.write("\n")
            
            return downloaded
    except urllib.error.URLError as e:
        raise RuntimeError(f"Failed to download {url}: {e}")


def _download_license(license_url: str, dest_dir: Path, filename: str = "LICENSE.txt") -> Optional[Path]:
    """Download license file to destination directory."""
    import urllib.request
    import urllib.error
    
    try:
        license_path = dest_dir / filename
        with urllib.request.urlopen(license_url, timeout=30) as response:
            content = response.read()
            license_path.write_bytes(content)
        return license_path
    except Exception:
        return None


def _asset_subdir(kind: str) -> str:
    if kind == "font":
        return "fonts"
    if kind == "css":
        return "css"
    if kind in {"icon", "icons"}:
        return "icons"
    return kind or "assets"


def _vendor_relative_path(kind: str, filename: str) -> str:
    return f"vendor/{_asset_subdir(kind)}/{filename}"


def _license_filename(package_name: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in {"-", "_"} else "-" for ch in package_name.lower())
    return f"LICENSE.{safe}.txt"


def _write_license_notice(
    dest_dir: Path,
    package_name: str,
    version: str,
    license_name: Optional[str],
    license_url: Optional[str],
) -> Optional[Path]:
    if not license_name:
        return None
    license_path = dest_dir / _license_filename(package_name)
    lines = [
        f"Asset: {package_name}",
        f"Version: {version}",
        f"License: {license_name}",
    ]
    if license_url:
        lines.append(f"License-URL: {license_url}")
    lines.append("")
    lines.append("This file is generated by fullbleed assets install.")
    license_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return license_path


def _resolve_remote_asset(name: str) -> Optional[Dict]:
    """Resolve a remote asset reference."""
    # Strip @ prefix if present
    key = name.lstrip("@").lower()
    
    # Handle version suffixes
    version = None
    if "@" in key:
        key, version = key.split("@", 1)
    
    # Check remote registry
    if key in REMOTE_ASSETS:
        asset = REMOTE_ASSETS[key].copy()
        asset["name"] = key
        asset["remote"] = True
        if version:
            asset["version"] = version
        return asset
    
    return None


def _resolve_builtin_asset(name: str) -> Optional[Dict]:
    """Resolve a builtin asset reference like bootstrap or @bootstrap."""
    raw = name.strip()
    if not raw:
        return None
    if raw.startswith("@"):
        raw = raw[1:]
    key = raw.lower()
    
    # Handle version suffixes like bootstrap@5.0.0 or @bootstrap@5.0.0.
    version = None
    if "@" in key:
        key, version = key.split("@", 1)
    
    # Normalize common aliases
    aliases = {
        "bootstrap5": "bootstrap",
        "bootstrap5.0.0": "bootstrap",
        "bootstrap-icons1.11.3": "bootstrap-icons",
        "bootstrapicons": "bootstrap-icons",
        "noto-sans-regular": "noto-sans",
    }
    key = aliases.get(key, key)
    
    if key not in BUILTIN_ASSETS:
        return None
    
    asset = BUILTIN_ASSETS[key].copy()
    asset["name"] = key
    asset["version"] = version or asset["default"]
    asset["builtin"] = True
    return asset


def _compute_file_hash(path: Path) -> str:
    """Compute SHA256 hash of a file."""
    hasher = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def _get_file_info(path: Path) -> Dict:
    """Get detailed file info for nutritional label."""
    stat = path.stat()
    return {
        "path": str(path),
        "name": path.name,
        "size_bytes": stat.st_size,
        "sha256": _compute_file_hash(path),
    }


def cmd_assets_list(args):
    """List installed asset packages."""
    import fullbleed_assets
    
    show_available = getattr(args, "available", False)
    
    # List builtin assets
    builtins = []
    for name, info in BUILTIN_ASSETS.items():
        builtins.append({
            "name": name,
            "version": info["default"],
            "kind": info["kind"],
            "description": info["description"],
            "install_refs": [name, f"@{name}"],
            "source": "builtin",
        })
    
    # List cached packages
    from .cache import list_cached_packages
    cached = list_cached_packages()
    for pkg in cached:
        pkg["source"] = "cache"
    
    all_packages = builtins + cached
    
    # List available remote packages if requested
    remote_packages = []
    if show_available:
        for name, info in REMOTE_ASSETS.items():
            remote_packages.append({
                "name": name,
                "version": info.get("version", "latest"),
                "kind": info["kind"],
                "description": info["description"],
                "license": info.get("license"),
                "source": "remote",
            })
    
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.assets_list.v1",
            "packages": all_packages,
        }
        if show_available:
            result["available"] = remote_packages
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write("Installed asset packages:\n")
        for pkg in all_packages:
            source_tag = f"[{pkg['source']}]"
            sys.stdout.write(f"  {pkg['name']}@{pkg.get('version', '?')} {source_tag} - {pkg.get('description', pkg.get('kind', ''))}\n")
        sys.stdout.write(
            "\nBuiltin install refs: use plain or @ refs "
            "(bootstrap, bootstrap-icons, noto-sans).\n"
        )
        
        if show_available:
            sys.stdout.write("\nAvailable remote packages:\n")
            for pkg in remote_packages:
                license_tag = f"({pkg.get('license', '?')})"
                sys.stdout.write(f"  {pkg['name']}@{pkg['version']} {license_tag} - {pkg['description']}\n")


def cmd_assets_info(args):
    """Show detailed info (nutritional label) for an asset package."""
    package_name = args.package.lstrip("@")
    
    # Check if it's a builtin
    builtin = _resolve_builtin_asset(f"@{package_name}")
    
    if builtin:
        import fullbleed_assets
        
        # Get the actual file path
        if builtin["name"] == "bootstrap":
            file_path = fullbleed_assets.asset_path("bootstrap.min.css")
        elif builtin["name"] == "bootstrap-icons":
            file_path = fullbleed_assets.asset_path("icons/bootstrap-icons.svg")
        elif builtin["name"] == "noto-sans":
            file_path = fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")
        else:
            file_path = None
        
        info = {
            "name": builtin["name"],
            "version": builtin["version"],
            "kind": builtin["kind"],
            "description": builtin.get("description", ""),
            "source": "builtin",
            "license": builtin.get("license"),
            "license_url": builtin.get("license_url"),
        }
        
        if file_path and Path(file_path).exists():
            file_info = _get_file_info(Path(file_path))
            info["files"] = [file_info]
            info["total_size_bytes"] = file_info["size_bytes"]
        
        if getattr(args, "json", False):
            result = {"schema": "fullbleed.assets_info.v1", **info}
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            _print_nutritional_label(info)
        return
    
    # Check cached packages
    from .cache import list_cached_packages
    for pkg in list_cached_packages():
        if pkg["name"] == package_name:
            pkg_path = Path(pkg["path"])
            files = [_get_file_info(f) for f in pkg_path.rglob("*") if f.is_file()]
            info = {
                "name": pkg["name"],
                "version": pkg["version"],
                "source": "cache",
                "files": files,
                "total_size_bytes": sum(f["size_bytes"] for f in files),
            }
            if getattr(args, "json", False):
                result = {"schema": "fullbleed.assets_info.v1", **info}
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                _print_nutritional_label(info)
            return
    
    # Package not found
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": "PACKAGE_NOT_FOUND",
            "message": f"Package not found: {package_name}",
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] Package not found: {package_name}\n")
    raise SystemExit(1)


def _print_nutritional_label(info: Dict):
    """Print a human-readable nutritional label for an asset."""
    sys.stdout.write(f"\n{'=' * 50}\n")
    sys.stdout.write(f"  {info['name']}@{info.get('version', '?')}\n")
    sys.stdout.write(f"{'=' * 50}\n\n")
    
    if "description" in info:
        sys.stdout.write(f"  Description: {info['description']}\n")
    if "kind" in info:
        sys.stdout.write(f"  Kind:        {info['kind']}\n")
    if "license" in info:
        sys.stdout.write(f"  License:     {info['license']}\n")
    if "source" in info:
        sys.stdout.write(f"  Source:      {info['source']}\n")
    if "total_size_bytes" in info:
        size_kb = info["total_size_bytes"] / 1024
        sys.stdout.write(f"  Size:        {size_kb:.1f} KB\n")
    
    if "files" in info and info["files"]:
        sys.stdout.write(f"\n  Files ({len(info['files'])}):\n")
        for f in info["files"][:5]:  # Show first 5 files
            size_kb = f["size_bytes"] / 1024
            sys.stdout.write(f"    - {f['name']} ({size_kb:.1f} KB)\n")
        if len(info["files"]) > 5:
            sys.stdout.write(f"    ... and {len(info['files']) - 5} more\n")
    
    sys.stdout.write("\n")


def cmd_assets_install(args):
    """Install an asset package to the local cache or vendor directory."""
    package_ref = args.package
    is_json = getattr(args, "json", False)
    use_global = getattr(args, "global_", False)  # --global flag
    explicit_vendor = getattr(args, "vendor", None)
    
    # Project-aware destination (cargo/npm style)
    project_root = detect_project_root()
    install_scope = "global_cache"
    if explicit_vendor:
        vendor_dir = explicit_vendor
        install_scope = "custom_vendor"
    elif use_global or not project_root:
        vendor_dir = None  # Use global cache
        install_scope = "global_cache"
    else:
        # Default to ./vendor/ when in a project
        vendor_dir = str(project_root / "vendor")
        install_scope = "project_vendor"
        if not is_json:
            sys.stdout.write(f"[project] Installing to {vendor_dir} (use --global for cache)\n")
    if not is_json and install_scope == "global_cache" and not use_global and not explicit_vendor:
        sys.stdout.write(
            "[global] No fullbleed project markers found; installing to global cache. "
            "Run `fullbleed init` for default per-project vendoring.\n"
        )
    
    # Try builtin first
    builtin = _resolve_builtin_asset(package_ref)
    if builtin:
        import fullbleed_assets
        
        # Determine source and destination
        if builtin["name"] == "bootstrap":
            src_path = Path(fullbleed_assets.asset_path("bootstrap.min.css"))
        elif builtin["name"] == "bootstrap-icons":
            src_path = Path(fullbleed_assets.asset_path("icons/bootstrap-icons.svg"))
        elif builtin["name"] == "noto-sans":
            src_path = Path(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))
        else:
            raise ValueError(f"Unknown builtin: {builtin['name']}")
        
        if vendor_dir:
            dest_dir = Path(vendor_dir) / _asset_subdir(builtin["kind"])
        else:
            dest_dir = get_package_cache_path(builtin["name"], builtin["version"])
        
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest_path = dest_dir / src_path.name
        shutil.copy2(src_path, dest_path)
        license_path = _write_license_notice(
            dest_dir=dest_dir,
            package_name=builtin["name"],
            version=builtin["version"],
            license_name=builtin.get("license"),
            license_url=builtin.get("license_url"),
        )
        
        result = {
            "name": builtin["name"],
            "version": builtin["version"],
            "installed_to": str(dest_path),
            "size_bytes": dest_path.stat().st_size,
            "sha256": _compute_file_hash(dest_path),
            "license": builtin.get("license"),
            "license_file": str(license_path) if license_path else None,
            "source": "builtin",
            "install_scope": install_scope,
            "project_detected": project_root is not None,
        }
        
        if is_json:
            output = {"schema": "fullbleed.assets_install.v1", "ok": True, **result}
            sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(f"[ok] Installed {builtin['name']}@{builtin['version']} to {dest_path}\n")
        
        # Auto-update assets.lock.json when writing to vendor in project context.
        if project_root and vendor_dir:
            lock_kind = "svg" if builtin["kind"] in {"icon", "icons"} else builtin["kind"]
            lock_entry = {
                "name": builtin["name"],
                "version": builtin["version"],
                "kind": lock_kind,
                "sha256": result["sha256"],
                "path": _vendor_relative_path(builtin["kind"], src_path.name),
            }
            _update_lock_file(project_root, lock_entry)
            if not is_json:
                sys.stdout.write(f"[lock] Updated assets.lock.json\n")
            
            # Auto-register in report.py
            updated_report = _update_report_py(project_root, lock_entry)
            if updated_report and not is_json:
                sys.stdout.write(f"[report] Registered {builtin['name']} in report.py\n")

        return
    
    # Try remote asset
    remote = _resolve_remote_asset(package_ref)
    if remote:
        if vendor_dir:
            dest_dir = Path(vendor_dir) / _asset_subdir(remote["kind"])
        else:
            dest_dir = get_package_cache_path(remote["name"], remote["version"])
        
        dest_path = dest_dir / remote["filename"]
        vendor_rel_path = _vendor_relative_path(remote["kind"], remote["filename"])
        license_path = dest_dir / _license_filename(remote["name"])
        
        # Check if already cached
        if dest_path.exists():
            if not is_json:
                sys.stdout.write(f"[cached] {remote['name']}@{remote['version']} already installed\n")
            result = {
                "name": remote["name"],
                "version": remote["version"],
                "installed_to": str(dest_path),
                "size_bytes": dest_path.stat().st_size,
                "sha256": _compute_file_hash(dest_path),
                "license": remote.get("license"),
                "license_file": str(license_path) if license_path.exists() else None,
                "source": "cache",
                "cached": True,
                "install_scope": install_scope,
                "project_detected": project_root is not None,
            }
            if is_json:
                output = {"schema": "fullbleed.assets_install.v1", "ok": True, **result}
                sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
            return
        
        # Download the asset
        if not is_json:
            sys.stdout.write(f"[downloading] {remote['name']}@{remote['version']} from Google Fonts\n")
        
        try:
            bytes_downloaded = _download_file(remote["url"], dest_path, show_progress=not is_json)
        except Exception as e:
            if is_json:
                result = {
                    "schema": "fullbleed.error.v1",
                    "ok": False,
                    "code": "DOWNLOAD_FAILED",
                    "message": str(e),
                }
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                sys.stderr.write(f"[error] Download failed: {e}\n")
            raise SystemExit(1)
        
        # Download license
        downloaded_license_path = None
        if "license_url" in remote:
            downloaded_license_path = _download_license(
                remote["license_url"],
                dest_dir,
                filename=_license_filename(remote["name"]),
            )
        if not downloaded_license_path:
            downloaded_license_path = _write_license_notice(
                dest_dir=dest_dir,
                package_name=remote["name"],
                version=remote["version"],
                license_name=remote.get("license"),
                license_url=remote.get("license_url"),
            )
        
        result = {
            "name": remote["name"],
            "version": remote["version"],
            "installed_to": str(dest_path),
            "size_bytes": dest_path.stat().st_size,
            "sha256": _compute_file_hash(dest_path),
            "license": remote.get("license"),
            "license_file": str(downloaded_license_path) if downloaded_license_path else None,
            "source": "remote",
            "install_scope": install_scope,
            "project_detected": project_root is not None,
        }
        
        if is_json:
            output = {"schema": "fullbleed.assets_install.v1", "ok": True, **result}
            sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(f"[ok] Installed {remote['name']}@{remote['version']} ({bytes_downloaded // 1024} KB)\n")
            sys.stdout.write(f"     License: {remote.get('license', 'unknown')}\n")
            sys.stdout.write(f"     Path: {dest_path}\n")
        
        # Auto-update assets.lock.json when writing to vendor in project context.
        if project_root and vendor_dir:
            lock_kind = "svg" if remote["kind"] in {"icon", "icons"} else remote["kind"]
            lock_entry = {
                "name": remote["name"],
                "version": remote["version"],
                "kind": lock_kind,
                "sha256": result["sha256"],
                "path": vendor_rel_path,
            }
            _update_lock_file(project_root, lock_entry)
            if not is_json:
                sys.stdout.write(f"[lock] Updated assets.lock.json\n")
            
            # Auto-register in report.py
            updated_report = _update_report_py(project_root, lock_entry)
            if updated_report and not is_json:
                sys.stdout.write(f"[report] Registered {remote['name']} in report.py\n")

        return
    
    # Package not found
    if is_json:
        result = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": "PACKAGE_NOT_FOUND",
            "message": f"Unknown package: {package_ref}",
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] Unknown package: {package_ref}\n")
        sys.stderr.write("  Available builtin assets:\n")
        for name in BUILTIN_ASSETS:
            sys.stderr.write(f"    @{name}\n")
        sys.stderr.write("  Available remote assets:\n")
        for name in list(REMOTE_ASSETS.keys())[:10]:
            sys.stderr.write(f"    {name}\n")
        if len(REMOTE_ASSETS) > 10:
            sys.stderr.write(f"    ... and {len(REMOTE_ASSETS) - 10} more\n")
    raise SystemExit(1)


def _verify_lock_constraints(package_name: str, version: Optional[str], observed_hashes: List[str], lock_path: str):
    violations = []
    path = Path(lock_path)
    if not path.exists():
        return [{"code": "LOCK_NOT_FOUND", "message": f"Lock file not found: {path}"}]
    try:
        lock_data = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [{"code": "LOCK_INVALID", "message": f"Failed to parse lock file: {exc}"}]

    packages = lock_data.get("packages")
    if not isinstance(packages, list):
        return [{"code": "LOCK_INVALID", "message": "Lock file has no 'packages' list"}]

    pkg_entry = next((p for p in packages if p.get("name") == package_name), None)
    if not pkg_entry:
        return [{"code": "PACKAGE_NOT_LOCKED", "message": f"Package not present in lock: {package_name}"}]

    expected_version = pkg_entry.get("version")
    if version and expected_version and version != expected_version:
        violations.append(
            {
                "code": "VERSION_MISMATCH",
                "message": f"Version mismatch for {package_name}: observed={version} expected={expected_version}",
            }
        )

    expected_hashes = []
    for file_entry in pkg_entry.get("files", []) or []:
        file_hash = file_entry.get("sha256")
        if file_hash:
            expected_hashes.append(file_hash)
    if expected_hashes and observed_hashes and set(expected_hashes).isdisjoint(set(observed_hashes)):
        violations.append(
            {
                "code": "HASH_MISMATCH",
                "message": f"No observed hash matched lock file for {package_name}",
            }
        )
    return violations


def cmd_assets_verify(args):
    """Verify an asset package by rendering its test case."""
    package_name = args.package.lstrip("@")
    lock_path = getattr(args, "lock", None)
    strict = bool(getattr(args, "strict", False))

    # For now, just verify the package exists and compute hashes.
    builtin = _resolve_builtin_asset(f"@{package_name}")
    cached = []
    if not builtin:
        from .cache import list_cached_packages
        cached = [p for p in list_cached_packages() if p["name"] == package_name]
        if not cached:
            if getattr(args, "json", False):
                result = {
                    "schema": "fullbleed.error.v1",
                    "ok": False,
                    "code": "PACKAGE_NOT_FOUND",
                    "message": f"Package not found: {package_name}",
                }
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                sys.stderr.write(f"[error] Package not found: {package_name}\n")
            raise SystemExit(1)

    observed_hashes = []
    result = {
        "schema": "fullbleed.assets_verify.v1",
        "ok": True,
        "name": package_name,
        "checks": ["exists"],
    }

    if builtin:
        import fullbleed_assets
        if builtin["name"] == "bootstrap":
            file_path = fullbleed_assets.asset_path("bootstrap.min.css")
        elif builtin["name"] == "bootstrap-icons":
            file_path = fullbleed_assets.asset_path("icons/bootstrap-icons.svg")
        elif builtin["name"] == "noto-sans":
            file_path = fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")
        else:
            raise ValueError(f"Unknown builtin package in verify: {builtin['name']}")

        path = Path(file_path)
        if not path.exists():
            result = {
                "schema": "fullbleed.assets_verify.v1",
                "ok": False,
                "name": package_name,
                "error": "File not found",
            }
            if getattr(args, "json", False):
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                sys.stderr.write(f"[error] {package_name}: file not found\n")
            raise SystemExit(1)

        file_hash = _compute_file_hash(path)
        observed_hashes.append(file_hash)
        result.update(
            {
                "version": builtin["version"],
                "sha256": file_hash,
                "size_bytes": path.stat().st_size,
                "checks": ["exists", "readable", "hash"],
            }
        )
    else:
        pkg_path = Path(cached[0]["path"])
        files = [f for f in pkg_path.rglob("*") if f.is_file()]
        for file_path in files:
            observed_hashes.append(_compute_file_hash(file_path))
        result.update(
            {
                "version": cached[0]["version"],
                "files": len(files),
                "checks": ["exists", "hash"],
            }
        )

    if lock_path:
        violations = _verify_lock_constraints(package_name, result.get("version"), observed_hashes, lock_path)
        result["lock_file"] = str(Path(lock_path))
        result["lock_ok"] = len(violations) == 0
        if violations:
            result["ok"] = False
            result["violations"] = violations
            if strict:
                result["strict"] = True
        else:
            result["checks"].append("lock")

    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        if result["ok"]:
            if "sha256" in result:
                sys.stdout.write(
                    f"[ok] {package_name}@{result.get('version')} verified (sha256: {result['sha256'][:16]}...)\n"
                )
            else:
                sys.stdout.write(f"[ok] {package_name}@{result.get('version')} verified\n")
        else:
            sys.stderr.write(f"[error] {package_name}: lock verification failed\n")
            for violation in result.get("violations", []):
                sys.stderr.write(f"  - {violation.get('code')}: {violation.get('message')}\n")
    if strict and not result["ok"]:
        raise SystemExit(1)


def _update_lock_file(project_root: Path, entry: dict) -> bool:
    """Update assets.lock.json with a single package entry."""
    lock_path = project_root / "assets.lock.json"
    
    try:
        if lock_path.exists():
            with open(lock_path, "r", encoding="utf-8") as f:
                lock_data = json.load(f)
        else:
            lock_data = {
                "schema": 1,
                "packages": [],
            }
            
        # Update or add
        existing_idx = next(
            (i for i, p in enumerate(lock_data["packages"]) if p["name"] == entry["name"]),
            None
        )
        
        # Ensure entry has valid structure
        path_lower = str(entry["path"]).lower()
        inferred_kind = (
            "font"
            if any(path_lower.endswith(ext) for ext in (".ttf", ".otf", ".woff", ".woff2"))
            else "svg"
            if path_lower.endswith(".svg")
            else "css"
        )
        pkg_entry = {
            "name": entry["name"],
            "version": entry["version"],
            "kind": entry.get("kind", inferred_kind),
            "files": [
                {
                    "path": entry["path"],
                    "sha256": entry["sha256"],
                }
            ],
        }

        if existing_idx is not None:
            lock_data["packages"][existing_idx] = pkg_entry
        else:
            lock_data["packages"].append(pkg_entry)
            
        with open(lock_path, "w", encoding="utf-8") as f:
            json.dump(lock_data, f, ensure_ascii=True, indent=2)
            
        return True
    except Exception as e:
        sys.stderr.write(f"[warning] Failed to update lock file: {e}\n")
        return False


def cmd_assets_lock(args):
    """Create or update assets.lock.json from current project dependencies."""
    lock_path = Path(args.output) if hasattr(args, "output") and args.output else Path("assets.lock.json")
    
    # Read existing lock file if it exists
    if lock_path.exists():
        with open(lock_path, "r", encoding="utf-8") as f:
            lock_data = json.load(f)
    else:
        lock_data = {
            "schema": 1,
            "packages": [],
        }
    
    # Add any specified packages
    packages_to_add = getattr(args, "add", None) or []
    
    for pkg_ref in packages_to_add:
        builtin = _resolve_builtin_asset(pkg_ref)
        if builtin:
            import fullbleed_assets
            
            if builtin["name"] == "bootstrap":
                file_path = Path(fullbleed_assets.asset_path("bootstrap.min.css"))
            elif builtin["name"] == "bootstrap-icons":
                file_path = Path(fullbleed_assets.asset_path("icons/bootstrap-icons.svg"))
            elif builtin["name"] == "noto-sans":
                file_path = Path(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))
            else:
                continue
            
            file_hash = _compute_file_hash(file_path)
            
            pkg_entry = {
                "name": builtin["name"],
                "version": builtin["version"],
                "kind": builtin["kind"],
                "files": [
                    {
                        "path": _vendor_relative_path(builtin["kind"], file_path.name),
                        "sha256": file_hash,
                    }
                ],
            }
            
            # Update or add
            existing_idx = next(
                (i for i, p in enumerate(lock_data["packages"]) if p["name"] == builtin["name"]),
                None
            )
            if existing_idx is not None:
                lock_data["packages"][existing_idx] = pkg_entry
            else:
                lock_data["packages"].append(pkg_entry)
    
    # Write lock file
    with open(lock_path, "w", encoding="utf-8") as f:
        json.dump(lock_data, f, ensure_ascii=True, indent=2)
    
    result = {
        "schema": "fullbleed.assets_lock.v1",
        "ok": True,
        "path": str(lock_path),
        "packages": len(lock_data["packages"]),
    }
    
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] Wrote {lock_path} with {len(lock_data['packages'])} package(s)\n")


def _update_report_py(project_root: Path, asset_info: dict) -> bool:
    """Auto-register an installed asset in report.py (AssetBundle pattern)."""
    report_path = project_root / "report.py"
    if not report_path.exists():
        return False
    
    try:
        content = report_path.read_text(encoding="utf-8")
        original_content = content
        asset_path = asset_info["path"]
        
        # Determine asset type
        is_font = asset_path.endswith(".ttf") or asset_path.endswith(".otf") or asset_path.endswith(".woff2")
        is_css = asset_path.endswith(".css")
        is_svg = asset_path.endswith(".svg")
        
        kind = "font" if is_font else "css" if is_css else "svg" if is_svg else None
        if not kind:
            return False
            
        # Check if already registered
        if f'bundle.add_file("{asset_path}"' in content:
            return False
            
        # Find the injection point: "bundle = fullbleed.AssetBundle()"
        marker = "bundle = fullbleed.AssetBundle()"
        if marker not in content:
            return False
            
        lines = content.splitlines()
        insertion_idx = -1
        
        # Find line with marker
        for i, line in enumerate(lines):
            if marker in line:
                insertion_idx = i + 1
                break
        
        if insertion_idx == -1:
            return False
            
        # Skip past comments/blanks immediately following
        while insertion_idx < len(lines):
            line = lines[insertion_idx].strip()
            # If line is empty or comment, skip
            if not line or line.startswith("#"):
                insertion_idx += 1
            else:
                break
                
        # Insert the new line
        # Use proper indentation (4 spaces)
        new_line = f'    bundle.add_file("{asset_path}", "{kind}")'
        lines.insert(insertion_idx, new_line)
        
        content = "\n".join(lines) + "\n"
        
        if content != original_content:
            report_path.write_text(content, encoding="utf-8")
            return True
            
    except Exception as e:
        sys.stderr.write(f"[warning] Failed to update report.py: {e}\n")
        return False
        
    return False

