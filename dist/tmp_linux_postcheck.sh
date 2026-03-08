set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cd /mnt/c/dev/workbench/fullbleed-official
PYBIN=/usr/bin/python3
SITE=/tmp/fullbleed_ci068_site
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed --version
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed doctor --strict --json
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed compliance --strict --json
PYTHONPATH="$SITE" "$PYBIN" tools/check_license_integrity.py --json