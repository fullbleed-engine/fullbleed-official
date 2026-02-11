# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Cache management for fullbleed asset packages.

Provides platform-appropriate cache directory handling and pruning.
"""
import json
import os
import shutil
import sys
from pathlib import Path
from datetime import datetime


def get_cache_dir() -> Path:
    """Get the platform-appropriate cache directory for fullbleed.
    
    - Windows: %LOCALAPPDATA%\\fullbleed\\cache
    - macOS: ~/Library/Caches/fullbleed
    - Linux: ~/.cache/fullbleed (XDG_CACHE_HOME if set)
    """
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA")
        if base:
            return Path(base) / "fullbleed" / "cache"
        return Path.home() / "AppData" / "Local" / "fullbleed" / "cache"
    elif sys.platform == "darwin":
        return Path.home() / "Library" / "Caches" / "fullbleed"
    else:
        xdg = os.environ.get("XDG_CACHE_HOME")
        if xdg:
            return Path(xdg) / "fullbleed"
        return Path.home() / ".cache" / "fullbleed"


def ensure_cache_dir() -> Path:
    """Ensure cache directory exists and return its path."""
    cache_dir = get_cache_dir()
    cache_dir.mkdir(parents=True, exist_ok=True)
    return cache_dir


def get_package_cache_path(package_name: str, version: str) -> Path:
    """Get the cache path for a specific package version."""
    cache_dir = ensure_cache_dir()
    return cache_dir / "packages" / package_name / version


def list_cached_packages():
    """List all cached packages with their versions and sizes."""
    packages_dir = get_cache_dir() / "packages"
    if not packages_dir.exists():
        return []
    
    result = []
    for pkg_dir in packages_dir.iterdir():
        if not pkg_dir.is_dir():
            continue
        for version_dir in pkg_dir.iterdir():
            if not version_dir.is_dir():
                continue
            # Calculate total size
            total_size = sum(f.stat().st_size for f in version_dir.rglob("*") if f.is_file())
            # Get last access time
            try:
                mtime = max(f.stat().st_mtime for f in version_dir.rglob("*") if f.is_file())
                last_access = datetime.fromtimestamp(mtime).isoformat()
            except ValueError:
                last_access = None
            
            result.append({
                "name": pkg_dir.name,
                "version": version_dir.name,
                "path": str(version_dir),
                "size_bytes": total_size,
                "last_access": last_access,
            })
    
    return result


def prune_cache(max_age_days: int = 90, dry_run: bool = False):
    """Remove cached packages older than max_age_days.
    
    Returns list of removed packages.
    """
    from datetime import timedelta
    
    packages = list_cached_packages()
    now = datetime.now()
    cutoff = now - timedelta(days=max_age_days)
    
    removed = []
    for pkg in packages:
        if pkg["last_access"]:
            last_access = datetime.fromisoformat(pkg["last_access"])
            if last_access < cutoff:
                if not dry_run:
                    shutil.rmtree(pkg["path"], ignore_errors=True)
                removed.append(pkg)
    
    return removed


def cmd_cache_dir(args):
    """Print the cache directory path."""
    cache_dir = get_cache_dir()
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.cache_dir.v1",
            "path": str(cache_dir),
            "exists": cache_dir.exists(),
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(str(cache_dir) + "\n")


def cmd_cache_prune(args):
    """Prune old cached packages."""
    max_age = getattr(args, "max_age_days", 90)
    dry_run = getattr(args, "dry_run", False)
    
    removed = prune_cache(max_age_days=max_age, dry_run=dry_run)
    
    if getattr(args, "json", False):
        result = {
            "schema": "fullbleed.cache_prune.v1",
            "dry_run": dry_run,
            "removed_count": len(removed),
            "removed": removed,
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        if dry_run:
            sys.stdout.write(f"[dry-run] would remove {len(removed)} packages\n")
        else:
            sys.stdout.write(f"[ok] removed {len(removed)} packages\n")
        for pkg in removed:
            sys.stdout.write(f"  - {pkg['name']}@{pkg['version']}\n")
