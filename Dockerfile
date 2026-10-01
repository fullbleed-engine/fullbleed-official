# Optional local MCP distribution. The Python/Rust core does not require Docker.
FROM python:3.12-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e

LABEL org.opencontainers.image.title="Fullbleed MCP" \
      org.opencontainers.image.description="Local stdio tools for Fullbleed PDF Engine" \
      org.opencontainers.image.source="https://github.com/fullbleed-engine/fullbleed-official" \
      org.opencontainers.image.licenses="MIT"

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

COPY packages/fullbleed-mcp /build/fullbleed-mcp
RUN python -m pip install --no-cache-dir --only-binary=:all: --no-deps \
        --require-hashes -r /build/fullbleed-mcp/container-requirements.txt \
    && python -m pip install --no-cache-dir --no-deps /build/fullbleed-mcp \
    && python -m pip check \
    && rm -rf /build \
    && mkdir /workspace \
    && chown 10001:10001 /workspace

USER 10001:10001
WORKDIR /workspace
ENTRYPOINT ["fullbleed-mcp"]
CMD ["--root", "/workspace"]
