from __future__ import annotations

import importlib.machinery
import importlib.util
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


def _without_fullbleed_editable_finders(finders: list[object]) -> list[object]:
    return [
        finder
        for finder in finders
        if not getattr(finder, "_fullbleed_editable_finder", False)
    ]


def _remove_fullbleed_editable_finders() -> None:
    sys.meta_path[:] = _without_fullbleed_editable_finders(list(sys.meta_path))


def _installed_fullbleed_native_candidates(
    search_path: list[str] | None = None,
) -> list[Path]:
    local_package = (PYTHON_SRC / "fullbleed").resolve()
    candidates: list[Path] = []
    seen: set[Path] = set()
    for entry in sys.path if search_path is None else search_path:
        if not entry:
            continue
        try:
            package = (Path(entry).resolve() / "fullbleed").resolve()
        except OSError:
            continue
        if package == local_package:
            continue
        for suffix in importlib.machinery.EXTENSION_SUFFIXES:
            candidate = package / f"_fullbleed{suffix}"
            if candidate.is_file() and candidate not in seen:
                candidates.append(candidate)
                seen.add(candidate)
    return candidates


def _load_installed_fullbleed_native() -> bool:
    name = "fullbleed._fullbleed"
    candidates = _installed_fullbleed_native_candidates()
    if not candidates:
        return False
    candidate = candidates[0]
    spec = importlib.util.spec_from_file_location(name, candidate)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load installed Fullbleed extension: {candidate}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        if sys.modules.get(name) is module:
            sys.modules.pop(name, None)
        raise
    return True


def _ensure_fullbleed_native_stub() -> None:
    package = PYTHON_SRC / "fullbleed"
    if any(
        (package / f"_fullbleed{suffix}").is_file()
        for suffix in importlib.machinery.EXTENSION_SUFFIXES
    ):
        return
    if "fullbleed._fullbleed" in sys.modules:
        return
    if _load_installed_fullbleed_native():
        return
    stub = types.ModuleType("fullbleed._fullbleed")
    stub.__all__ = []
    stub.__doc__ = "Stubbed native module for pure-Python UI tests."
    sys.modules["fullbleed._fullbleed"] = stub


_remove_fullbleed_editable_finders()
_prefer_local_fullbleed_package()
_ensure_fullbleed_native_stub()
