set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cd /mnt/c/dev/workbench/fullbleed-official
PYBIN=/usr/bin/python3
SITE=/tmp/fullbleed_ci068_site
rm -rf "$SITE"
mkdir -p "$SITE"
"$PYBIN" -m pip install --user --upgrade pip maturin
"$PYBIN" tools/generate_css_parity_status.py --check --json
"$PYBIN" tools/generate_css_parity_status.py --out output/css_parity_status.ci.linux.json --json
"$PYBIN" -m maturin build --release --features python
WHEEL=$(ls -t target/wheels/fullbleed-*.whl | head -n 1)
echo "Using wheel: $WHEEL"
"$PYBIN" -m pip install --target "$SITE" --force-reinstall "$WHEEL"
PYTHONPATH="$SITE" "$PYBIN" tools/run_css_fixture_suite.py --json
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed --version
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed doctor --strict --json
PYTHONPATH="$SITE" "$PYBIN" -m fullbleed compliance --strict --json
PYTHONPATH="$SITE" "$PYBIN" tools/check_license_integrity.py --json