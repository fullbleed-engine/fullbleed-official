set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
cd /mnt/c/dev/workbench/fullbleed-official
PYBIN=/home/keenan/.local/bin/python3.11
SITE=/tmp/fullbleed_ci068_site311
rm -rf "$SITE"
mkdir -p "$SITE"
maturin build --release --features python -i "$PYBIN"
WHEEL=$(ls -t target/wheels/fullbleed-*.whl | head -n 1)
echo "Using wheel: $WHEEL"
"$PYBIN" -m pip install --target "$SITE" --force-reinstall "$WHEEL"
PYTHONPATH="$SITE" "$PYBIN" tools/run_css_fixture_suite.py --json