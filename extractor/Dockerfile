# syntax=docker/dockerfile:1
FROM python:3.12-slim-bookworm

COPY --from=ghcr.io/astral-sh/uv:0.13.9 /uv /uvx /bin/

# No apt packages are required: docling pulls opencv-python-headless and torch bundles its
# own runtime libs (verified by importing docling and converting a PDF in this image).

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON_DOWNLOADS=never \
    UV_NO_DEV=1 \
    PATH="/app/.venv/bin:$PATH" \
    PYTHONUNBUFFERED=1 \
    DATA_DIR=/data \
    DOCLING_ARTIFACTS_PATH=/opt/docling-models \
    HF_HUB_OFFLINE=1

WORKDIR /app

# 1) dependencies only (cached until pyproject.toml / uv.lock change)
COPY pyproject.toml uv.lock .python-version ./
RUN uv sync --frozen --no-dev --no-install-project

# 2) Docling model weights (layout, tableformer, rapidocr) baked into the image so the
#    container starts without network access. HF_HUB_OFFLINE only applies at runtime.
RUN HF_HUB_OFFLINE=0 docling-tools models download layout tableformer rapidocr \
        -o "$DOCLING_ARTIFACTS_PATH"

# 3) application code
COPY app ./app
RUN uv sync --frozen --no-dev

# Non-root user. The uid/gid is fixed so the shared `data` volume can be written by
# other services running with the same uid.
RUN groupadd --gid 10001 app \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin app \
    && mkdir -p /data \
    && chown app:app /data
USER app

EXPOSE 8000
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000", "--workers", "1"]
