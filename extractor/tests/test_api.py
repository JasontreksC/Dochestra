import csv
import json
import time
from pathlib import Path

from fastapi.testclient import TestClient

HEALTH_TIMEOUT_S = 300


def wait_ready(client: TestClient) -> None:
    deadline = time.monotonic() + HEALTH_TIMEOUT_S
    while time.monotonic() < deadline:
        r = client.get("/health")
        if r.status_code == 200:
            assert r.json() == {"status": "ok"}
            return
        assert r.status_code == 503
        assert r.json() == {"status": "loading"}
        time.sleep(1)
    raise AssertionError("extractor did not become healthy in time")


def test_extract_sample_pdf(client: TestClient, data_dir: Path) -> None:
    wait_ready(client)
    out_dir = data_dir / "runs" / "r1" / "n1"
    r = client.post(
        "/extract",
        json={"pdf_path": str(data_dir / "bundles" / "sample.pdf"), "output_dir": str(out_dir)},
    )
    assert r.status_code == 200, r.text
    body = r.json()
    assert set(body) == {"md", "json", "tables", "images"}

    root = out_dir.resolve()
    md, js = Path(body["md"]), Path(body["json"])
    assert md == root / "md.md" and js == root / "json.json"
    assert md.read_text(encoding="utf-8").strip()
    assert isinstance(json.loads(js.read_text(encoding="utf-8")), dict)

    assert len(body["tables"]) >= 1
    assert len(body["images"]) >= 1
    for i, t in enumerate(body["tables"]):
        assert Path(t) == root / "tables" / f"{i:04}.csv"
    for i, img in enumerate(body["images"]):
        assert Path(img) == root / "images" / f"{i:04}.jpg"
    for p in [body["md"], body["json"], *body["tables"], *body["images"]]:
        path = Path(p)
        assert path.is_absolute() and path.is_file()
        assert path.is_relative_to(root)

    rows = list(csv.reader(Path(body["tables"][0]).open(encoding="utf-8")))
    assert rows[0] == ["Quarter", "Revenue", "Cost", "Profit"]
    assert len(rows) >= 4
    assert Path(body["images"][0]).read_bytes()[:3] == b"\xff\xd8\xff"  # JPEG magic


def test_outside_data_dir_rejected(client: TestClient, data_dir: Path) -> None:
    wait_ready(client)
    ok_pdf = str(data_dir / "bundles" / "sample.pdf")
    r = client.post("/extract", json={"pdf_path": "/etc/passwd", "output_dir": str(data_dir / "o")})
    assert r.status_code == 400 and "detail" in r.json()
    r = client.post("/extract", json={"pdf_path": ok_pdf, "output_dir": "/tmp/elsewhere"})
    assert r.status_code == 400 and "detail" in r.json()
    r = client.post(
        "/extract",
        json={"pdf_path": f"{data_dir}/../x.pdf", "output_dir": str(data_dir / "o")},
    )
    assert r.status_code == 400


def test_missing_pdf_rejected(client: TestClient, data_dir: Path) -> None:
    wait_ready(client)
    r = client.post(
        "/extract",
        json={"pdf_path": str(data_dir / "nope.pdf"), "output_dir": str(data_dir / "o")},
    )
    assert r.status_code == 400 and "detail" in r.json()


def test_invalid_body_is_422(client: TestClient) -> None:
    assert client.post("/extract", json={"pdf_path": "x"}).status_code == 422


def test_loading_state_before_startup(tmp_path: Path) -> None:
    from app.main import create_app

    c = TestClient(create_app(tmp_path))  # no lifespan -> converter never starts
    r = c.get("/health")
    assert r.status_code == 503 and r.json() == {"status": "loading"}
    r = c.post("/extract", json={"pdf_path": "x", "output_dir": "y"})
    assert r.status_code == 503 and "detail" in r.json()
