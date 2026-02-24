from __future__ import annotations

import sys
import types
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PYTHON_SRC = ROOT / "python"

if str(PYTHON_SRC) not in sys.path:
    sys.path.insert(0, str(PYTHON_SRC))
else:
    sys.path.remove(str(PYTHON_SRC))
    sys.path.insert(0, str(PYTHON_SRC))


def _prefer_local_fullbleed_package() -> None:
    loaded = sys.modules.get("fullbleed")
    if loaded is None:
        return
    mod_file = getattr(loaded, "__file__", "") or ""
    if str(PYTHON_SRC / "fullbleed") in mod_file:
        return
    for name in list(sys.modules):
        if name == "fullbleed" or name.startswith("fullbleed."):
            sys.modules.pop(name, None)


def _ensure_fullbleed_native_stub() -> None:
    native_glob = list((PYTHON_SRC / "fullbleed").glob("_fullbleed.*"))
    if native_glob:
        return
    if "fullbleed._fullbleed" in sys.modules:
        return
    stub = types.ModuleType("fullbleed._fullbleed")
    stub.__all__ = []
    stub.__doc__ = "Stubbed native module for pure-Python UI tests."
    sys.modules["fullbleed._fullbleed"] = stub


_prefer_local_fullbleed_package()
_ensure_fullbleed_native_stub()
