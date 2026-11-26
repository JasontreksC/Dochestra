from pathlib import Path

import pytest

from app.paths import resolve_under


def test_inside_path_is_resolved(tmp_path: Path) -> None:
    f = tmp_path / "a" / "b.pdf"
    f.parent.mkdir()
    f.touch()
    assert resolve_under(str(f), tmp_path) == f.resolve()


def test_nonexistent_inside_path_allowed(tmp_path: Path) -> None:
    target = tmp_path / "runs" / "r1" / "n1"
    assert resolve_under(str(target), tmp_path) == target.resolve()


def test_data_dir_itself_allowed(tmp_path: Path) -> None:
    assert resolve_under(str(tmp_path), tmp_path) == tmp_path.resolve()


def test_dotdot_escape_rejected(tmp_path: Path) -> None:
    data = tmp_path / "data"
    data.mkdir()
    with pytest.raises(ValueError):
        resolve_under(str(data / ".." / "secret.pdf"), data)


def test_absolute_outside_rejected(tmp_path: Path) -> None:
    data = tmp_path / "data"
    data.mkdir()
    with pytest.raises(ValueError):
        resolve_under("/etc/passwd", data)


def test_sibling_prefix_dir_rejected(tmp_path: Path) -> None:
    data = tmp_path / "data"
    sibling = tmp_path / "data-evil"
    data.mkdir()
    sibling.mkdir()
    with pytest.raises(ValueError):
        resolve_under(str(sibling / "x.pdf"), data)


def test_symlink_escape_rejected(tmp_path: Path) -> None:
    data = tmp_path / "data"
    outside = tmp_path / "outside"
    data.mkdir()
    outside.mkdir()
    (outside / "x.pdf").touch()
    (data / "link").symlink_to(outside, target_is_directory=True)
    with pytest.raises(ValueError):
        resolve_under(str(data / "link" / "x.pdf"), data)


def test_symlink_inside_allowed(tmp_path: Path) -> None:
    real = tmp_path / "real"
    real.mkdir()
    (real / "x.pdf").touch()
    (tmp_path / "alias").symlink_to(real, target_is_directory=True)
    assert resolve_under(str(tmp_path / "alias" / "x.pdf"), tmp_path) == (real / "x.pdf").resolve()
