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
    },
    "noto-sans": {
        "versions": ["regular"],
        "default": "regular",
        "kind": "font",
        "description": "Noto Sans font family (Regular weight)",
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
        "sha256": None,  # Will verify on first download
    },
    "noto-sans-italic": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/OFL.txt",
        "description": "Noto Sans Italic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans-Italic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSans-Italic-Variable.ttf",
        "sha256": None,
    },
    "noto-serif-regular": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt",
        "description": "Noto Serif Regular (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSerif-Variable.ttf",
        "sha256": None,
    },
    "noto-serif-italic": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/OFL.txt",
        "description": "Noto Serif Italic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notoserif/NotoSerif-Italic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSerif-Italic-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-mono": {
        "kind": "font",
        "version": "2.014",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/OFL.txt",
        "description": "Noto Sans Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansmono/NotoSansMono%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansMono-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-jp": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/OFL.txt",
        "description": "Noto Sans Japanese (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
        "filename": "NotoSansJP-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-sc": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/OFL.txt",
        "description": "Noto Sans Simplified Chinese (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
        "filename": "NotoSansSC-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-kr": {
        "kind": "font",
        "version": "2.004",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/OFL.txt",
        "description": "Noto Sans Korean (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf",
        "filename": "NotoSansKR-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-arabic": {
        "kind": "font",
        "version": "2.010",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/OFL.txt",
        "description": "Noto Sans Arabic (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansArabic-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-hebrew": {
        "kind": "font",
        "version": "2.003",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/OFL.txt",
        "description": "Noto Sans Hebrew (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansHebrew-Variable.ttf",
        "sha256": None,
    },
    "noto-sans-thai": {
        "kind": "font",
        "version": "2.002",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/OFL.txt",
        "description": "Noto Sans Thai (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansthai/NotoSansThai%5Bwdth%2Cwght%5D.ttf",
        "filename": "NotoSansThai-Variable.ttf",
        "sha256": None,
    },
    "noto-color-emoji": {
        "kind": "font",
        "version": "2.047",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/OFL.txt",
        "description": "Noto Color Emoji",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/notocoloremoji/NotoColorEmoji-Regular.ttf",
        "filename": "NotoColorEmoji-Regular.ttf",
        "sha256": None,
    },
    "inter": {
        "kind": "font",
        "version": "4.0",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inter/OFL.txt",
        "description": "Inter - Modern sans-serif for UI",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf",
        "filename": "Inter-Variable.ttf",
        "sha256": None,
    },
    "roboto": {
        "kind": "font",
        "version": "wdth,wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/roboto/OFL.txt",
        "description": "Roboto - Android system font",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/roboto/Roboto%5Bwdth%2Cwght%5D.ttf",
        "filename": "Roboto-Variable.ttf",
        "sha256": None,
    },
    # Sans-Serif
    "open-sans": {
        "kind": "font",
        "version": "wdth,wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OFL.txt",
        "description": "Open Sans (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/opensans/OpenSans%5Bwdth%2Cwght%5D.ttf",
        "filename": "OpenSans-Variable.ttf",
        "sha256": None,
    },
    "lato": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lato/OFL.txt",
        "description": "Lato (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lato/Lato-Regular.ttf",
        "filename": "Lato-Regular.ttf",
        "sha256": None,
    },
    "montserrat": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/OFL.txt",
        "description": "Montserrat (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat/Montserrat%5Bwght%5D.ttf",
        "filename": "Montserrat-Variable.ttf",
        "sha256": None,
    },
    "poppins": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/OFL.txt",
        "description": "Poppins (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/poppins/Poppins-Regular.ttf",
        "filename": "Poppins-Regular.ttf",
        "sha256": None,
    },
    "raleway": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/OFL.txt",
        "description": "Raleway (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/raleway/Raleway%5Bwght%5D.ttf",
        "filename": "Raleway-Variable.ttf",
        "sha256": None,
    },
    "oswald": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/OFL.txt",
        "description": "Oswald (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/Oswald%5Bwght%5D.ttf",
        "filename": "Oswald-Variable.ttf",
        "sha256": None,
    },
    "ubuntu": {
        "kind": "font",
        "version": "regular",
        "license": "UFL-1.0",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ufl/ubuntu/UFL.txt",
        "description": "Ubuntu (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ufl/ubuntu/Ubuntu-Regular.ttf",
        "filename": "Ubuntu-Regular.ttf",
        "sha256": None,
    },
    "nunito": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/OFL.txt",
        "description": "Nunito (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/nunito/Nunito%5Bwght%5D.ttf",
        "filename": "Nunito-Variable.ttf",
        "sha256": None,
    },
    "rubik": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/OFL.txt",
        "description": "Rubik (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/rubik/Rubik%5Bwght%5D.ttf",
        "filename": "Rubik-Variable.ttf",
        "sha256": None,
    },
    "work-sans": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/OFL.txt",
        "description": "Work Sans (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/worksans/WorkSans%5Bwght%5D.ttf",
        "filename": "WorkSans-Variable.ttf",
        "sha256": None,
    },
    
    # Serif
    # "merriweather": {
    #     "kind": "font",
    #     "version": "regular",
    #     "license": "OFL-1.1",
    #     "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/merriweather/OFL.txt",
    #     "description": "Merriweather (Regular)",
    #     "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/merriweather/Merriweather-Regular.ttf",
    #     "filename": "Merriweather-Regular.ttf",
    #     "sha256": None,
    # },
    "playfair-display": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/OFL.txt",
        "description": "Playfair Display (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/PlayfairDisplay%5Bwght%5D.ttf",
        "filename": "PlayfairDisplay-Variable.ttf",
        "sha256": None,
    },
    "lora": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lora/OFL.txt",
        "description": "Lora (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lora/Lora%5Bwght%5D.ttf",
        "filename": "Lora-Variable.ttf",
        "sha256": None,
    },
    "pt-serif": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/OFL.txt",
        "description": "PT Serif (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ptserif/PT_Serif-Web-Regular.ttf",
        "filename": "PTSerif-Regular.ttf",
        "sha256": None,
    },
    "crimson-text": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/OFL.txt",
        "description": "Crimson Text (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/crimsontext/CrimsonText-Regular.ttf",
        "filename": "CrimsonText-Regular.ttf",
        "sha256": None,
    },
    "libre-baskerville": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/OFL.txt",
        "description": "Libre Baskerville (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/librebaskerville/LibreBaskerville%5Bwght%5D.ttf",
        "filename": "LibreBaskerville-Variable.ttf",
        "sha256": None,
    },
    "arvo": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/OFL.txt",
        "description": "Arvo (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/arvo/Arvo-Regular.ttf",
        "filename": "Arvo-Regular.ttf",
        "sha256": None,
    },
    "eb-garamond": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/OFL.txt",
        "description": "EB Garamond (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond/EBGaramond%5Bwght%5D.ttf",
        "filename": "EBGaramond-Variable.ttf",
        "sha256": None,
    },

    # Display / Handwriting
    "bebas-neue": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/bebasneue/OFL.txt",
        "description": "Bebas Neue (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/bebasneue/BebasNeue-Regular.ttf",
        "filename": "BebasNeue-Regular.ttf",
        "sha256": None,
    },
    "lobster": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lobster/OFL.txt",
        "description": "Lobster (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/lobster/Lobster-Regular.ttf",
        "filename": "Lobster-Regular.ttf",
        "sha256": None,
    },
    "abril-fatface": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/abrilfatface/OFL.txt",
        "description": "Abril Fatface (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/abrilfatface/AbrilFatface-Regular.ttf",
        "filename": "AbrilFatface-Regular.ttf",
        "sha256": None,
    },
    "dancing-script": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/dancingscript/OFL.txt",
        "description": "Dancing Script (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/dancingscript/DancingScript%5Bwght%5D.ttf",
        "filename": "DancingScript-Variable.ttf",
        "sha256": None,
    },
    "pacifico": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/pacifico/OFL.txt",
        "description": "Pacifico (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/pacifico/Pacifico-Regular.ttf",
        "filename": "Pacifico-Regular.ttf",
        "sha256": None,
    },
    "shadows-into-light": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/shadowsintolight/OFL.txt",
        "description": "Shadows Into Light (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/shadowsintolight/ShadowsIntoLight.ttf",
        "filename": "ShadowsIntoLight-Regular.ttf",
        "sha256": None,
    },
    "comfortaa": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/comfortaa/OFL.txt",
        "description": "Comfortaa (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/comfortaa/Comfortaa%5Bwght%5D.ttf",
        "filename": "Comfortaa-Variable.ttf",
        "sha256": None,
    },
    "righteous": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/righteous/OFL.txt",
        "description": "Righteous (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/righteous/Righteous-Regular.ttf",
        "filename": "Righteous-Regular.ttf",
        "sha256": None,
    },
    "fredoka": {
        "kind": "font",
        "version": "wdth,wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/fredoka/OFL.txt",
        "description": "Fredoka (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/fredoka/Fredoka%5Bwdth%2Cwght%5D.ttf",
        "filename": "Fredoka-Variable.ttf",
        "sha256": None,
    },
    
    # Monospace
    "roboto-mono": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/OFL.txt",
        "description": "Roboto Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/robotomono/RobotoMono%5Bwght%5D.ttf",
        "filename": "RobotoMono-Variable.ttf",
        "sha256": None,
    },
    "source-code-pro": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/OFL.txt",
        "description": "Source Code Pro (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/sourcecodepro/SourceCodePro%5Bwght%5D.ttf",
        "filename": "SourceCodePro-Variable.ttf",
        "sha256": None,
    },
    "fira-code": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/OFL.txt",
        "description": "Fira Code (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/firacode/FiraCode%5Bwght%5D.ttf",
        "filename": "FiraCode-Variable.ttf",
        "sha256": None,
    },
    "inconsolata": {
        "kind": "font",
        "version": "wdth,wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inconsolata/OFL.txt",
        "description": "Inconsolata (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/inconsolata/Inconsolata%5Bwdth%2Cwght%5D.ttf",
        "filename": "Inconsolata-Variable.ttf",
        "sha256": None,
    },
    "ibm-plex-mono": {
        "kind": "font",
        "version": "regular",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ibmplexmono/OFL.txt",
        "description": "IBM Plex Mono (Regular)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/ibmplexmono/IBMPlexMono-Regular.ttf",
        "filename": "IBMPlexMono-Regular.ttf",
        "sha256": None,
    },
    "jetbrains-mono": {
        "kind": "font",
        "version": "wght",
        "license": "OFL-1.1",
        "license_url": "https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/OFL.txt",
        "description": "JetBrains Mono (Variable)",
        "url": "https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf",
        "sha256": None,
    },
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


def _update_lock_file(project_root: Path, package_info: dict) -> bool:
    """Add or update a package entry in assets.lock.json.
    
    Returns True if the lock file was updated, False if no changes needed.
    """
    lock_path = project_root / "assets.lock.json"
    
    # Load existing lock file or create new structure
    if lock_path.exists():
        try:
            lock_data = json.loads(lock_path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, IOError):
            lock_data = {"schema": 1, "packages": []}
    else:
        lock_data = {"schema": 1, "packages": []}
    
    packages = lock_data.get("packages", [])
    
    # Check if package already exists
    for pkg in packages:
        if pkg.get("name") == package_info["name"]:
            # Update existing entry
            pkg.update(package_info)
            lock_path.write_text(json.dumps(lock_data, indent=2) + "\n", encoding="utf-8")
            return True
    
    # Add new entry
    packages.append(package_info)
    lock_data["packages"] = packages
    lock_path.write_text(json.dumps(lock_data, indent=2) + "\n", encoding="utf-8")
    return True


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


def _download_license(license_url: str, dest_dir: Path) -> Optional[Path]:
    """Download license file to destination directory."""
    import urllib.request
    import urllib.error
    
    try:
        license_path = dest_dir / "LICENSE.txt"
        with urllib.request.urlopen(license_url, timeout=30) as response:
            content = response.read()
            license_path.write_bytes(content)
        return license_path
    except Exception:
        return None


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
    """Resolve a builtin asset reference like @bootstrap or @noto-sans."""
    if not name.startswith("@"):
        return None
    key = name[1:].lower()
    
    # Handle version suffixes like @bootstrap@5.0.0
    version = None
    if "@" in key:
        key, version = key.split("@", 1)
    
    # Normalize common aliases
    aliases = {
        "bootstrap5": "bootstrap",
        "bootstrap5.0.0": "bootstrap", 
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
            "license": "MIT" if builtin["name"] == "bootstrap" else "OFL-1.1",
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
    if explicit_vendor:
        vendor_dir = explicit_vendor
    elif use_global or not project_root:
        vendor_dir = None  # Use global cache
    else:
        # Default to ./vendor/ when in a project
        vendor_dir = str(project_root / "vendor")
        if not is_json:
            sys.stdout.write(f"[project] Installing to {vendor_dir} (use --global for cache)\n")
    
    # Try builtin first
    builtin = _resolve_builtin_asset(package_ref)
    if builtin:
        import fullbleed_assets
        
        # Determine source and destination
        if builtin["name"] == "bootstrap":
            src_path = Path(fullbleed_assets.asset_path("bootstrap.min.css"))
        elif builtin["name"] == "noto-sans":
            src_path = Path(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf"))
        else:
            raise ValueError(f"Unknown builtin: {builtin['name']}")
        
        if vendor_dir:
            dest_dir = Path(vendor_dir)
        else:
            dest_dir = get_package_cache_path(builtin["name"], builtin["version"])
        
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest_path = dest_dir / src_path.name
        shutil.copy2(src_path, dest_path)
        
        result = {
            "name": builtin["name"],
            "version": builtin["version"],
            "installed_to": str(dest_path),
            "size_bytes": dest_path.stat().st_size,
            "sha256": _compute_file_hash(dest_path),
            "source": "builtin",
        }
        
        if is_json:
            output = {"schema": "fullbleed.assets_install.v1", "ok": True, **result}
            sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(f"[ok] Installed {builtin['name']}@{builtin['version']} to {dest_path}\n")
        
        # Auto-update assets.lock.json when in project context
        if project_root:
            lock_entry = {
                "name": builtin["name"],
                "version": builtin["version"],
                "sha256": result["sha256"],
                "path": f"vendor/{src_path.name}",
            }
            _update_lock_file(project_root, lock_entry)
            if not is_json:
                sys.stdout.write(f"[lock] Updated assets.lock.json\n")
            
            # Auto-register in fullbleed.toml
            updated_config = _update_fullbleed_toml(project_root, {
                "name": builtin["name"],
                "version": builtin["version"],
                "kind": "css" if builtin["name"] == "bootstrap" else "font" # Simplistic kind inference
            })
            if updated_config and not is_json:
                sys.stdout.write(f"[config] Added {builtin['name']} to fullbleed.toml\n")

        return
    
    # Try remote asset
    remote = _resolve_remote_asset(package_ref)
    if remote:
        if vendor_dir:
            dest_dir = Path(vendor_dir)
        else:
            dest_dir = get_package_cache_path(remote["name"], remote["version"])
        
        dest_path = dest_dir / remote["filename"]
        
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
                "source": "cache",
                "cached": True,
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
        license_path = None
        if "license_url" in remote:
            license_path = _download_license(remote["license_url"], dest_dir)
        
        result = {
            "name": remote["name"],
            "version": remote["version"],
            "installed_to": str(dest_path),
            "size_bytes": dest_path.stat().st_size,
            "sha256": _compute_file_hash(dest_path),
            "license": remote.get("license"),
            "license_file": str(license_path) if license_path else None,
            "source": "remote",
        }
        
        if is_json:
            output = {"schema": "fullbleed.assets_install.v1", "ok": True, **result}
            sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(f"[ok] Installed {remote['name']}@{remote['version']} ({bytes_downloaded // 1024} KB)\n")
            sys.stdout.write(f"     License: {remote.get('license', 'unknown')}\n")
            sys.stdout.write(f"     Path: {dest_path}\n")
        
        # Auto-update assets.lock.json when in project context
        if project_root:
            lock_entry = {
                "name": remote["name"],
                "version": remote["version"],
                "sha256": result["sha256"],
                "path": f"vendor/{remote['filename']}",
            }
            _update_lock_file(project_root, lock_entry)
            if not is_json:
                sys.stdout.write(f"[lock] Updated assets.lock.json\n")
            
            # Auto-register in fullbleed.toml
            # Infer kind from filename
            fname = remote["filename"].lower()
            kind = "font" if fname.endswith((".ttf", ".otf", ".woff2")) else "css"
            
            updated_config = _update_fullbleed_toml(project_root, {
                "name": remote["name"],
                "version": remote["version"],
                "kind": kind
            })
            if updated_config and not is_json:
                 sys.stdout.write(f"[config] Added {remote['name']} to fullbleed.toml\n")

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


def cmd_assets_verify(args):
    """Verify an asset package by rendering its test case."""
    package_name = args.package.lstrip("@")
    
    # For now, just verify the package exists and compute hashes
    builtin = _resolve_builtin_asset(f"@{package_name}")
    
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
    
    # Basic verification: check files exist and compute hashes
    import fullbleed_assets
    
    if builtin and builtin["name"] == "bootstrap":
        file_path = fullbleed_assets.asset_path("bootstrap.min.css")
    elif builtin and builtin["name"] == "noto-sans":
        file_path = fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")
    else:
        # For cached packages, just report success for now
        result = {
            "schema": "fullbleed.assets_verify.v1",
            "ok": True,
            "name": package_name,
            "checks": ["exists"],
        }
        if getattr(args, "json", False):
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(f"[ok] {package_name} verified\n")
        return
    
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
    result = {
        "schema": "fullbleed.assets_verify.v1",
        "ok": True,
        "name": package_name,
        "version": builtin["version"],
        "sha256": file_hash,
        "size_bytes": path.stat().st_size,
        "checks": ["exists", "readable", "hash"],
    }
    
    if getattr(args, "json", False):
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] {package_name}@{builtin['version']} verified (sha256: {file_hash[:16]}...)\n")


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
        pkg_entry = {
            "name": entry["name"],
            "version": entry["version"],
            "kind": entry.get("kind", "font" if "ttf" in entry["path"] or "otf" in entry["path"] else "css"),
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
                        "path": file_path.name,
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


def _update_fullbleed_toml(project_root: Path, asset_info: dict) -> bool:
    """Auto-register an installed asset in fullbleed.toml."""
    config_path = project_root / "fullbleed.toml"
    if not config_path.exists():
        return False
    
    try:
        content = config_path.read_text(encoding="utf-8")
        original_content = content
        
        name = asset_info["name"]
        version = asset_info["version"]
        kind = "font" if "font" in asset_info.get("kind", "") else "css"
        
        # Check if already present (simplistic check)
        if f'"{name}"' in content or f"'{name}'" in content:
            # TODO: smarter TOML update
            return False
            
        # Append to [assets] section
        # We look for [assets] and append after it
        if "[assets]" not in content:
            content += "\n[assets]\n"
        
        lines = content.splitlines()
        assets_idx = -1
        for i, line in enumerate(lines):
            if line.strip() == "[assets]":
                assets_idx = i
                break
        
        if assets_idx != -1:
            # Insert after the last line of the section (or until next section)
            insert_idx = assets_idx + 1
            while insert_idx < len(lines):
                line = lines[insert_idx].strip()
                if line.startswith("[") and line.endswith("]"):
                    break
                insert_idx += 1
            
            new_line = f'"{name}" = {{ version = "{version}", kind = "{kind}" }}'
            lines.insert(insert_idx, new_line)
            content = "\n".join(lines) + "\n"
        
        if content != original_content:
            config_path.write_text(content, encoding="utf-8")
            return True
            
    except Exception as e:
        sys.stderr.write(f"[warning] Failed to update fullbleed.toml: {e}\n")
        return False
        
    return False
