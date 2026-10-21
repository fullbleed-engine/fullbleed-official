# SPDX-License-Identifier: MIT
"""Static Bootstrap 5.0.0 components for the alpha document-authoring API.

The original preview import path remains supported. Both paths use the same
component objects and bundled stylesheet; this module adds no JavaScript or
separate copy of the kit.
"""
from ..ui.experimental import bootstrap as _bootstrap
from ..ui.experimental.bootstrap import *  # noqa: F401,F403

__all__ = list(_bootstrap.__all__)
