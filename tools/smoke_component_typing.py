# SPDX-License-Identifier: MIT
"""Check component call signatures through the installed wheel, using test-only mypy."""
from __future__ import annotations

import argparse
from importlib import metadata
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import fullbleed.ui.experimental as components


VALID = '''from fullbleed.ui.experimental import component, Node, Paragraph

@component(parts=("label", "note"))
def Notice(message: str, note: str | None = None) -> Node:
    return Paragraph(message, part="label")

valid: Node = Notice("Paid")
parts: tuple[str, ...] | None = Notice.parts
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[1] / "python"
    assert source not in Path(components.__file__).parents, "Use an installed candidate wheel with python -I"
    assert Path(components.__file__).with_name("py.typed").is_file(), "The wheel must carry its typing marker"
    environment = dict(os.environ)
    environment.pop("MYPYPATH", None)
    environment.pop("PYTHONPATH", None)
    with tempfile.TemporaryDirectory(prefix="fullbleed-component-types-") as directory:
        folder = Path(directory)
        (folder / "valid.py").write_text(VALID, encoding="utf-8")
        (folder / "invalid.py").write_text(VALID + '\nNotice(10)\nNotice("Paid", unexpected=True)\n', encoding="utf-8")

        def check(name):
            return subprocess.run(
                [sys.executable, "-I", "-m", "mypy", "--follow-imports=silent", "--no-incremental", name],
                cwd=folder, env=environment, capture_output=True, text=True, timeout=60,
            )

        valid, invalid = check("valid.py"), check("invalid.py")
        assert valid.returncode == 0, valid.stdout + valid.stderr
        assert invalid.returncode == 1, invalid.stdout + invalid.stderr
        assert invalid.stdout.count("[arg-type]") == 1, invalid.stdout
        assert invalid.stdout.count("[call-arg]") == 1, invalid.stdout
        assert "Found 2 errors" in invalid.stdout, invalid.stdout
    result = {"ok": True, "mypy_version": metadata.version("mypy"), "runtime": str(components.__file__),
              "valid_consumer": valid.stdout, "expected_invalid_consumer": invalid.stdout,
              "scope": "Installed component decorator call signatures and declared-parts metadata. Not full style/part-name completion."}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "expected_errors": 2}))


if __name__ == "__main__":
    main()
