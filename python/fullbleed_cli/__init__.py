# SPDX-License-Identifier: MIT
"""Version metadata for the public Fullbleed CLI package."""
from importlib.metadata import PackageNotFoundError, version


def _get_version():
    """Resolve the installed package version for CLI reporting."""
    for dist in ("fullbleed", "fullbleed-cli"):
        try:
            return version(dist)
        except PackageNotFoundError:
            continue
    return "0.0.0-dev"


__version__ = _get_version()
