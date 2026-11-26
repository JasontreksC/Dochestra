# syntax=docker/dockerfile:1
FROM python:3.12-slim-bookworm

COPY --from=ghcr.io/astral-sh/uv:0.13.9 /uv /uvx /bin/

# No apt packages are required: docling pulls opencv-python-headless and torch bundles its
# own runtime libs (verified by importing docling and converting a PDF in this image).
# CUDA (T-117): torch comes from the cu134 wheel index, which pulls the CUDA runtime as
# nvidia-* wheels, so no CUDA base image is needed. Only the host driver is required; the
# NVIDIA container runtime mounts it when the container is started with a GPU
# (`--gpus all` / compose device reservation). Without a GPU, EXTRACTOR_DEVICE=auto
# falls back to CPU.

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON_DOWNLOADS=never \
    UV_NO_DEV=1 \
    PATH="/app/.venv/bin:$PATH" \
    PYTHONUNBUFFERED=1 \
    DATA_DIR=/data \
    DOCLING_ARTIFACTS_PATH=/opt/docling-models \
    HF_HUB_OFFLINE=1 \
    NVIDIA_VISIBLE_DEVICES=all \
    NVIDIA_DRIVER_CAPABILITIES=compute,utility \
    EXTRACTOR_DEVICE=auto

WORKDIR /app

# uv's download/extract cache is ~6 GB for the CUDA wheels. It is mounted as tmpfs (RAM) for the
# sync steps only, so it never lands in an image layer and never touches the build cache on disk
# (a `type=cache` mount would keep the 6 GB on the host disk). UV_LINK_MODE=copy is required
# because the cache (tmpfs) and the venv (overlay) are different filesystems.
# 1) dependencies only (cached until pyproject.toml / uv.lock change)
COPY pyproject.toml uv.lock .python-version ./
RUN --mount=type=tmpfs,target=/root/.cache/uv,size=9g \
    uv sync --frozen --no-dev --no-install-project

# 2) Docling model weights (layout, tableformer, rapidocr) baked into the image so the
#    container starts without network access. HF_HUB_OFFLINE only applies at runtime.
RUN HF_HUB_OFFLINE=0 docling-tools models download layout tableformer rapidocr \
        -o "$DOCLING_ARTIFACTS_PATH"

# 3) application code
COPY app ./app
RUN --mount=type=tmpfs,target=/root/.cache/uv,size=9g \
    uv sync --frozen --no-dev

# Non-root user. The uid/gid is fixed so the shared `data` volume can be written by
# other services running with the same uid.
RUN groupadd --gid 10001 app \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin app \
    && mkdir -p /data \
    && chown app:app /data
USER app

EXPOSE 8000
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000", "--workers", "1"]
