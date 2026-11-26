"""FastAPI app: GET /health, POST /extract."""

import asyncio
import logging
import os
import threading
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

from docling.document_converter import DocumentConverter
from fastapi import FastAPI, HTTPException
from fastapi.concurrency import run_in_threadpool
from fastapi.responses import JSONResponse, Response

from app import converter as conv
from app.paths import resolve_under
from app.schemas import ExtractRequest, ExtractResponse, HealthResponse

log = logging.getLogger("extractor")


def create_app(data_dir: Path | None = None) -> FastAPI:
    base = (data_dir or Path(os.environ.get("DATA_DIR", "/data"))).resolve()

    ready = threading.Event()
    state: dict[str, DocumentConverter] = {}
    semaphore = asyncio.Semaphore(1)

    def load() -> None:
        try:
            c = conv.build_converter()
            conv.warm_up(c)
            state["converter"] = c
            ready.set()
            log.info("docling converter ready")
        except Exception:
            log.exception("docling converter initialization failed")

    @asynccontextmanager
    async def lifespan(_: FastAPI) -> AsyncIterator[None]:
        threading.Thread(target=load, name="docling-init", daemon=True).start()
        yield

    app = FastAPI(title="dochestra-extractor", lifespan=lifespan)

    @app.get("/health", response_model=HealthResponse)
    async def health() -> Response:
        if ready.is_set():
            return JSONResponse({"status": "ok"})
        return JSONResponse({"status": "loading"}, status_code=503)

    @app.post("/extract", response_model=ExtractResponse, response_model_by_alias=True)
    async def extract(req: ExtractRequest) -> ExtractResponse:
        if not ready.is_set():
            raise HTTPException(status_code=503, detail="converter is still loading")
        try:
            pdf = resolve_under(req.pdf_path, base)
            out_dir = resolve_under(req.output_dir, base)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        if not pdf.is_file():
            raise HTTPException(status_code=400, detail=f"pdf not found: {req.pdf_path}")

        c = state["converter"]
        async with semaphore:
            try:
                return await run_in_threadpool(conv.convert, c, pdf, out_dir)
            except Exception as exc:
                log.exception("conversion failed: %s", pdf)
                raise HTTPException(
                    status_code=500, detail=f"{type(exc).__name__}: {exc}"[:500]
                ) from exc

    return app


app = create_app()
