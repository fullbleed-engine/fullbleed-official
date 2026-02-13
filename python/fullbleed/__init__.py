# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Public Python bindings for the Fullbleed PDF engine.

This package re-exports the Rust extension module symbols (`PdfEngine`,
`AssetBundle`, `WatermarkSpec`, and helpers), and adds process-local helpers for
commercial license attestation metadata used by CLI compliance tooling.
"""
import os

from . import _fullbleed as _ext
from ._fullbleed import *  # noqa: F401,F403

__doc__ = _ext.__doc__
if hasattr(_ext, "__all__"):
    __all__ = _ext.__all__

SPDX_LICENSE_EXPRESSION = "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial"
COMMERCIAL_LICENSE_ENV_KEYS = (
    "FULLBLEED_LICENSE_MODE",
    "FULLBLEED_COMMERCIAL_LICENSED",
    "FULLBLEED_COMMERCIAL_LICENSE_ID",
    "FULLBLEED_COMMERCIAL_LICENSE",
    "FULLBLEED_COMMERCIAL_LICENSE_FILE",
    "FULLBLEED_COMMERCIAL_COMPANY",
    "FULLBLEED_COMMERCIAL_TIER",
)


def activate_commercial_license(
    license_id=None,
    *,
    company=None,
    tier=None,
    license_file=None,
    licensed=True,
):
    """Set process-local environment markers for commercial license attestation.

    This affects only the current process and children spawned from it. It does
    not persist machine-wide state.
    """
    os.environ["FULLBLEED_LICENSE_MODE"] = "commercial"
    if licensed:
        os.environ["FULLBLEED_COMMERCIAL_LICENSED"] = "1"
    if license_id:
        os.environ["FULLBLEED_COMMERCIAL_LICENSE_ID"] = str(license_id)
        os.environ["FULLBLEED_COMMERCIAL_LICENSE"] = str(license_id)
    if company:
        os.environ["FULLBLEED_COMMERCIAL_COMPANY"] = str(company)
    if tier:
        os.environ["FULLBLEED_COMMERCIAL_TIER"] = str(tier)
    if license_file:
        os.environ["FULLBLEED_COMMERCIAL_LICENSE_FILE"] = str(license_file)


def clear_commercial_license():
    """Clear process-local commercial license environment markers."""
    for key in COMMERCIAL_LICENSE_ENV_KEYS:
        os.environ.pop(key, None)


def commercial_license_status():
    """Return a snapshot of process-local commercial license markers."""
    return {
        "mode": os.environ.get("FULLBLEED_LICENSE_MODE", "auto"),
        "licensed": os.environ.get("FULLBLEED_COMMERCIAL_LICENSED"),
        "license_id": os.environ.get("FULLBLEED_COMMERCIAL_LICENSE_ID")
        or os.environ.get("FULLBLEED_COMMERCIAL_LICENSE"),
        "license_file": os.environ.get("FULLBLEED_COMMERCIAL_LICENSE_FILE"),
        "company": os.environ.get("FULLBLEED_COMMERCIAL_COMPANY"),
        "tier": os.environ.get("FULLBLEED_COMMERCIAL_TIER"),
    }


_EXTRA_EXPORTS = [
    "SPDX_LICENSE_EXPRESSION",
    "COMMERCIAL_LICENSE_ENV_KEYS",
    "activate_commercial_license",
    "clear_commercial_license",
    "commercial_license_status",
]
if "__all__" in globals():
    __all__ = list(__all__) + _EXTRA_EXPORTS
else:
    __all__ = _EXTRA_EXPORTS
