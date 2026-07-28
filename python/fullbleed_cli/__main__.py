# SPDX-License-Identifier: MIT
"""Enable `python -m fullbleed_cli` invocation."""
from .cli import main


if __name__ == "__main__":
    raise SystemExit(main())
