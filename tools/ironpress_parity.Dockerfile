FROM rust:1.97-bookworm AS rust-toolchain

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/venv/bin:/usr/local/cargo/bin:${PATH} \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo

COPY --from=rust-toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust-toolchain /usr/local/rustup /usr/local/rustup

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        fonts-dejavu-core \
        fonts-liberation \
        fonts-noto-cjk \
        fontconfig \
        git \
        jq \
        python3 \
        python3-venv \
    && poppler_dir=$(mktemp -d) \
    && curl --fail --location --retry 3 \
        --output "$poppler_dir/liblcms2-2.deb" \
        https://archive.neon.kde.org/user/pool/main/l/lcms2/liblcms2-2_2.16-2+24.04+noble+release+build2_amd64.deb \
    && curl --fail --location --retry 3 \
        --output "$poppler_dir/libpoppler140.deb" \
        https://archive.neon.kde.org/user/pool/main/p/poppler/libpoppler140_24.08.0-1+24.04+noble+release+build15_amd64.deb \
    && curl --fail --location --retry 3 \
        --output "$poppler_dir/poppler-utils.deb" \
        https://archive.neon.kde.org/user/pool/main/p/poppler/poppler-utils_24.08.0-1+24.04+noble+release+build15_amd64.deb \
    && printf '%s  %s\n' \
        369ab216d40364743188a3df30b3a86285aede504ddde89eea9b1bab8dbcbda5 \
        "$poppler_dir/liblcms2-2.deb" \
        189bb9e6c22fa0f49f4ee8e802f62324a366ca776a52ebb8965fe1bb6affa448 \
        "$poppler_dir/libpoppler140.deb" \
        af3d09ab4a363949efba54e3a589888c032c7a1616a6039453f65e99e03f358e \
        "$poppler_dir/poppler-utils.deb" \
        | sha256sum --check --strict \
    && apt-get install --yes --no-install-recommends \
        "$poppler_dir/liblcms2-2.deb" \
        "$poppler_dir/libpoppler140.deb" \
        "$poppler_dir/poppler-utils.deb" \
    && printf '%s  %s\n' \
        b1f76a56605df368efd233e09faad3bd910e50c0d6556c616a7c0b0adebf6013 \
        /usr/bin/pdftoppm \
        | sha256sum --check --strict \
    && test "$(pdftoppm -v 2>&1 | head -n 1)" = "pdftoppm version 24.08.0" \
    && python3 -m venv /venv \
    && rm -rf "$poppler_dir" /var/lib/apt/lists/*

WORKDIR /ironpress
