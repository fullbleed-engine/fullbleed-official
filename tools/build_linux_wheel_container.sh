#!/bin/sh
# Build one Linux wheel inside a PyPA or rust-cross policy container.
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: build_linux_wheel_container.sh TARGET COMPATIBILITY OUTPUT_DIR" >&2
    exit 2
fi

fullbleed_target=$1
fullbleed_compatibility=$2
fullbleed_output=$3

fullbleed_python=""
for fullbleed_candidate in \
    /opt/python/cp310-cp310/bin/python \
    /usr/bin/python3 \
    /usr/local/bin/python3
do
    # Cross images may include a target-architecture CPython whose executable
    # bit is set even though the host runner has no binfmt/QEMU registration.
    # Use the first interpreter that can actually run the build backend.
    if [ -x "$fullbleed_candidate" ] \
        && "$fullbleed_candidate" -c 'import sys' >/dev/null 2>&1
    then
        fullbleed_python=$fullbleed_candidate
        break
    fi
done
if [ -z "$fullbleed_python" ]; then
    echo "fullbleed-build: no Python interpreter is available in the build image" >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    fullbleed_cargo_home=/tmp/fullbleed-cargo-home
    fullbleed_rustup_home=/tmp/fullbleed-rustup-home
    export CARGO_HOME=$fullbleed_cargo_home
    export RUSTUP_HOME=$fullbleed_rustup_home
    curl --proto '=https' --tlsv1.2 --silent --show-error --fail \
        https://sh.rustup.rs -o /tmp/fullbleed-rustup-init.sh
    sh /tmp/fullbleed-rustup-init.sh -y --profile minimal --default-toolchain stable
    PATH="$CARGO_HOME/bin:$PATH"
    export PATH
fi

if command -v rustup >/dev/null 2>&1; then
    rustup target add "$fullbleed_target"
fi

"$fullbleed_python" build_backend/fullbleed_build_backend.py wheel \
    --out "$fullbleed_output" \
    --target "$fullbleed_target" \
    --compatibility "$fullbleed_compatibility"
