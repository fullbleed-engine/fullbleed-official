# SPDX-License-Identifier: MIT
"""Public Python bindings for the Fullbleed PDF engine."""

from . import _fullbleed as _ext
from . import _abi
from ._abi import *  # noqa: F401,F403

__all__ = list(_abi.__all__)

# Preserve the historical ``fullbleed._fullbleed`` import surface while the
# native module itself stays intentionally tiny and stable-ABI-only.
for _name in _abi.__all__:
    setattr(_ext, _name, getattr(_abi, _name))
_ext.__all__ = list(_abi.__all__)

SPDX_LICENSE_EXPRESSION = "MIT"

__all__.append("SPDX_LICENSE_EXPRESSION")
