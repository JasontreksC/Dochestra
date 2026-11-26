"""Path validation helpers: every externally supplied path must live under DATA_DIR."""

from pathlib import Path


def resolve_under(path: str, data_dir: Path) -> Path:
    """Resolve ``path`` and ensure it lies under ``data_dir``.

    Symlinks and ``..`` segments are resolved before the check, so neither can be
    used to escape ``data_dir``. Raises ``ValueError`` when the path is outside.
    """
    base = data_dir.resolve()
    resolved = Path(path).resolve()
    if not resolved.is_relative_to(base):
        raise ValueError(f"path is outside DATA_DIR: {path}")
    return resolved
