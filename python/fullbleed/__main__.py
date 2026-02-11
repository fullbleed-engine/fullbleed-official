# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Enable `python -m fullbleed` invocation."""
from fullbleed_cli.cli import main

if __name__ == "__main__":
    raise SystemExit(main())
