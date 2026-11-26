"""Docling PDF conversion and artifact writing (CONTEXT §3.5 / §4.2)."""

import json
import logging
import os
from pathlib import Path

from docling.datamodel.base_models import InputFormat
from docling.datamodel.pipeline_options import PdfPipelineOptions
from docling.document_converter import DocumentConverter, PdfFormatOption

from app.schemas import ExtractResponse

log = logging.getLogger(__name__)


def _env_flag(name: str, default: bool = False) -> bool:
    return os.environ.get(name, str(default)).strip().lower() in {"1", "true", "yes", "on"}


def build_converter() -> DocumentConverter:
    """Create the Docling converter. Model weights are located via DOCLING_ARTIFACTS_PATH."""
    options = PdfPipelineOptions(
        do_ocr=_env_flag("EXTRACTOR_OCR", False),
        do_table_structure=True,
        generate_picture_images=True,
        images_scale=2.0,
    )
    return DocumentConverter(
        format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=options)}
    )


def warm_up(converter: DocumentConverter) -> None:
    """Load the PDF pipeline (and its models) eagerly."""
    converter.initialize_pipeline(InputFormat.PDF)


def convert(converter: DocumentConverter, pdf: Path, out_dir: Path) -> ExtractResponse:
    """Convert ``pdf`` and write md/json/tables/images under ``out_dir``."""
    out_dir.mkdir(parents=True, exist_ok=True)
    doc = converter.convert(pdf).document

    md_path = out_dir / "md.md"
    md_path.write_text(doc.export_to_markdown(), encoding="utf-8")

    json_path = out_dir / "json.json"
    json_path.write_text(json.dumps(doc.export_to_dict(), ensure_ascii=False), encoding="utf-8")

    table_paths: list[str] = []
    for table in doc.tables:
        frame = table.export_to_dataframe(doc=doc)
        tables_dir = out_dir / "tables"
        tables_dir.mkdir(exist_ok=True)
        path = tables_dir / f"{len(table_paths):04}.csv"
        frame.to_csv(path, index=False, encoding="utf-8")
        table_paths.append(str(path))

    image_paths: list[str] = []
    for picture in doc.pictures:
        image = picture.get_image(doc)
        if image is None:
            log.warning("picture without image data skipped (%s)", picture.self_ref)
            continue
        images_dir = out_dir / "images"
        images_dir.mkdir(exist_ok=True)
        path = images_dir / f"{len(image_paths):04}.jpg"
        image.convert("RGB").save(path, format="JPEG", quality=90)
        image_paths.append(str(path))

    return ExtractResponse(
        md=str(md_path), json=str(json_path), tables=table_paths, images=image_paths
    )
