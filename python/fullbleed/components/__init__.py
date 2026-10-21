# SPDX-License-Identifier: MIT
"""Document components: the intended public namespace for the 2.6 API.

This API is still in alpha. Ordinary functions, component-owned styles, explicit
themes and page geometry compile to Fullbleed HTML/CSS. The existing
``fullbleed.ui`` API remains supported; ``fullbleed.ui.experimental`` exposes
these same objects for compatibility with earlier previews.

Importing components does not construct a renderer, read fonts or load styles.
"""
from ..ui import experimental as _experimental
from ..ui.experimental import *  # noqa: F401,F403

__all__ = list(_experimental.__all__)
