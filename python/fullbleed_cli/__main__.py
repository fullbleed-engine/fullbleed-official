# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Enable `python -m fullbleed_cli` invocation."""
from .cli import main


if __name__ == "__main__":
    raise SystemExit(main())
