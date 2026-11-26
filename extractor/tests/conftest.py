import shutil
from collections.abc import Iterator
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.main import create_app

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session")
def sample_pdf() -> Path:
    return FIXTURES / "sample.pdf"


@pytest.fixture(scope="session")
def data_dir(tmp_path_factory: pytest.TempPathFactory, sample_pdf: Path) -> Path:
    d = tmp_path_factory.mktemp("data")
    (d / "bundles").mkdir()
    shutil.copy(sample_pdf, d / "bundles" / "sample.pdf")
    return d


@pytest.fixture(scope="session")
def client(data_dir: Path) -> Iterator[TestClient]:
    """TestClient with lifespan; the real Docling converter loads in the background."""
    with TestClient(create_app(data_dir)) as c:
        yield c
