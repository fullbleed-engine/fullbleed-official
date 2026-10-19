# SPDX-License-Identifier: MIT
from __future__ import annotations

from collections.abc import Iterable, Mapping, Set
from functools import lru_cache
import difflib
import re

from .._style import AuthoringError
from ._core import classes, stylesheet
from ._schema import BREAKPOINTS


@lru_cache(maxsize=1)
def class_names() -> frozenset[str]:
    """Class selectors from the pinned stylesheet, including responsive utilities."""
    css = re.sub(r"/\*.*?\*/", "", stylesheet(), flags=re.DOTALL)
    result = set()
    for selector in re.findall(r"([^{}]+)\{", css):
        # Values in attribute selectors are not class names.
        selector = re.sub(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'', "", selector)
        result.update(re.findall(r"\.([A-Za-z_][A-Za-z0-9_-]*)", selector))
    return frozenset(result)


def utilities(**values) -> str:
    """Build checked Bootstrap classes, for example p=3, mt={'md': 2}.

    Names use Bootstrap prefixes: d, p, m, text, bg, align_items, and so on.
    True means the bare prefix; None/False omit it. Ordered value collections
    combine multiple classes. Responsive mappings accept xs/default, the five
    named breakpoints, and print where the actual stylesheet provides it.
    """
    output = []
    available = class_names()

    def add(prefix, value, *, responsive=True):
        if value is None or value is False:
            return
        if isinstance(value, Mapping):
            if not responsive:
                raise AuthoringError("Responsive values cannot contain nested mappings", code="bootstrap.utility")
            for breakpoint, item in value.items():
                if breakpoint not in ("default", "xs", "print", *BREAKPOINTS):
                    raise AuthoringError(f"Unknown Bootstrap breakpoint {breakpoint!r}", code="bootstrap.breakpoint")
                add(prefix if breakpoint in {"default", "xs"} else prefix + "-" + breakpoint, item, responsive=False)
            return
        if isinstance(value, Iterable) and not isinstance(value, (str, bytes, bytearray, Set)):
            for item in value:
                if isinstance(item, (Mapping, Iterable)) and not isinstance(item, str):
                    raise AuthoringError("Utility collections contain scalar values", code="bootstrap.utility")
                add(prefix, item)
            return
        if value is not True and (isinstance(value, bool) or not isinstance(value, (str, int))):
            raise AuthoringError("Utility values must be strings, integers or True", code="bootstrap.utility")
        name = prefix if value is True or value == "" else prefix + "-" + str(value)
        if name not in available:
            matches = difflib.get_close_matches(name, sorted(available), n=3)
            hint = "; try " + ", ".join(matches) if matches else ""
            raise AuthoringError(f"Bootstrap 5.0.0 has no class {name!r}{hint}", code="bootstrap.utility")
        output.append(name)

    for prefix, value in values.items():
        add(prefix.replace("_", "-"), value)
    return classes(output)
