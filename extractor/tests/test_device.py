"""EXTRACTOR_DEVICE resolution and /health device reporting (T-117)."""

import logging
import time
from pathlib import Path

import pytest
import torch
from fastapi.testclient import TestClient

from app import converter as conv
from app.main import create_app
from app.schemas import HealthResponse


def _cuda(monkeypatch: pytest.MonkeyPatch, available: bool) -> None:
    monkeypatch.setattr(torch.cuda, "is_available", lambda: available)


@pytest.mark.parametrize(
    ("value", "available", "expected"),
    [
        ("auto", True, "cuda"),
        ("auto", False, "cpu"),
        ("cuda", True, "cuda"),
        ("cpu", True, "cpu"),
        ("cpu", False, "cpu"),
        (" CUDA ", True, "cuda"),
        ("", True, "cuda"),
    ],
)
def test_resolve_device(monkeypatch, value, available, expected) -> None:
    _cuda(monkeypatch, available)
    assert conv.resolve_device(value) == expected


def test_resolve_device_reads_env_default_auto(monkeypatch) -> None:
    monkeypatch.delenv(conv.DEVICE_ENV, raising=False)
    _cuda(monkeypatch, True)
    assert conv.resolve_device() == "cuda"
    _cuda(monkeypatch, False)
    assert conv.resolve_device() == "cpu"
    monkeypatch.setenv(conv.DEVICE_ENV, "cpu")
    _cuda(monkeypatch, True)
    assert conv.resolve_device() == "cpu"


def test_auto_fallback_logs_warning(monkeypatch, caplog) -> None:
    _cuda(monkeypatch, False)
    with caplog.at_level(logging.WARNING, logger=conv.log.name):
        assert conv.resolve_device("auto") == "cpu"
    assert any("falling back to CPU" in r.getMessage() for r in caplog.records)


def test_cuda_without_gpu_is_error(monkeypatch) -> None:
    _cuda(monkeypatch, False)
    with pytest.raises(conv.DeviceError, match="cuda"):
        conv.resolve_device("cuda")


@pytest.mark.parametrize("value", ["gpu", "mps", "cuda:0", "xpu"])
def test_invalid_device_is_error(monkeypatch, value) -> None:
    _cuda(monkeypatch, True)
    with pytest.raises(conv.DeviceError, match="invalid"):
        conv.resolve_device(value)


def test_health_schema_device_optional() -> None:
    assert HealthResponse(status="loading").device is None
    assert HealthResponse(status="ok", device="cuda").model_dump() == {
        "status": "ok",
        "device": "cuda",
    }


def _start_with_fake_converter(monkeypatch, tmp_path: Path, env: str, available: bool):
    """Run the app lifespan with a stub converter so no models are loaded."""
    calls: list[str] = []

    def fake_build(device: str = "cpu") -> object:
        calls.append(device)
        return object()

    monkeypatch.setenv(conv.DEVICE_ENV, env)
    _cuda(monkeypatch, available)
    monkeypatch.setattr(conv, "build_converter", fake_build)
    monkeypatch.setattr(conv, "warm_up", lambda c: None)
    return TestClient(create_app(tmp_path)), calls


def _wait(cond, timeout: float = 10.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if cond():
            return True
        time.sleep(0.05)
    return False


@pytest.mark.parametrize(
    ("env", "available", "device"), [("cpu", True, "cpu"), ("auto", True, "cuda")]
)
def test_health_reports_device(monkeypatch, tmp_path, env, available, device) -> None:
    client, calls = _start_with_fake_converter(monkeypatch, tmp_path, env, available)
    with client:
        assert _wait(lambda: client.get("/health").status_code == 200)
        r = client.get("/health")
        assert r.json() == {"status": "ok", "device": device}
    assert calls == [device]


def test_cuda_required_without_gpu_never_healthy(monkeypatch, tmp_path, caplog) -> None:
    client, calls = _start_with_fake_converter(monkeypatch, tmp_path, "cuda", False)
    with caplog.at_level(logging.ERROR, logger="extractor"), client:
        assert _wait(
            lambda: any("device selection failed" in r.getMessage() for r in caplog.records)
        )
        r = client.get("/health")
        assert r.status_code == 503 and r.json() == {"status": "loading"}
    assert calls == []
