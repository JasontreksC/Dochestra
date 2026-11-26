"""Pydantic request/response models (CONTEXT §4.2)."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class ExtractRequest(BaseModel):
    pdf_path: str
    output_dir: str


class ExtractResponse(BaseModel):
    """Keys are the extract node's output socket ids. Serialize with ``by_alias=True``."""

    model_config = ConfigDict(populate_by_name=True)

    md: str
    json_: str = Field(alias="json")
    tables: list[str]
    images: list[str]


class HealthResponse(BaseModel):
    status: Literal["ok", "loading"]
    device: Literal["cuda", "cpu"] | None = None  # present when status == "ok"
