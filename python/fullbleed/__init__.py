# SPDX-License-Identifier: MIT
"""Public Python bindings for the Fullbleed PDF engine.

This package re-exports the Rust extension module symbols (`PdfEngine`,
`AssetBundle`, `WatermarkSpec`, and helpers).
"""
from . import _fullbleed as _ext
from ._fullbleed import *  # noqa: F401,F403

__doc__ = _ext.__doc__
if hasattr(_ext, "__all__"):
    __all__ = _ext.__all__

SPDX_LICENSE_EXPRESSION = "MIT"

_EXTRA_EXPORTS = ["SPDX_LICENSE_EXPRESSION"]
if "__all__" in globals():
    __all__ = list(__all__) + _EXTRA_EXPORTS
else:
    __all__ = _EXTRA_EXPORTS
