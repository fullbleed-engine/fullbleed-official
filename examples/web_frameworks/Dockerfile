# SPDX-License-Identifier: MIT
# The digest pins the official multi-platform Python image, including amd64/arm64.
ARG PYTHON_IMAGE=python:3.14.8-slim-bookworm@sha256:48b13b003dda20b16f9442b8475aa05fe21bf6579a8c881db92ffb4d8fd20f83
FROM ${PYTHON_IMAGE}

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1
WORKDIR /app
COPY requirements-fastapi.txt requirements-docker.txt ./
# Fail clearly if a wheel is unavailable; never fall back to a source build.
RUN python -m pip install --only-binary=:all: --no-cache-dir \
    --report /app/pip-install.json -r requirements-docker.txt \
    && python -m pip check

COPY invoice.py demo.py fastapi_app.py LICENSE ./
COPY templates/ ./templates/
COPY static/ ./static/
COPY fonts/ ./fonts/

# Application files remain owned by root; the runtime only reads them.
USER 10001:10001
EXPOSE 8000
CMD ["python", "-m", "uvicorn", "fastapi_app:app", "--host", "0.0.0.0", "--port", "8000", "--no-proxy-headers"]
