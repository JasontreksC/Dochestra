//! `/data` path conventions (CONTEXT §3.5). All run-artifact paths are built here.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::types::ScalarType;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("invalid path segment {0:?}: only [A-Za-z0-9_-]{{1,64}} is allowed")]
    InvalidSegment(String),
    #[error("path {0:?} cannot be resolved")]
    Unresolvable(PathBuf),
    #[error("path {0:?} is outside the data directory")]
    OutsideDataDir(PathBuf),
}

/// Accepts only `[A-Za-z0-9_-]{1,64}`; client-chosen ids must pass this before becoming
/// path components.
pub fn safe_segment(segment: &str) -> Result<&str, StorageError> {
    let ok = (1..=64).contains(&segment.len())
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(segment)
    } else {
        Err(StorageError::InvalidSegment(segment.to_owned()))
    }
}

/// `{data_dir}/runs/{run_id}/{node_id}`
pub fn run_node_dir(data_dir: &Path, run_id: Uuid, node_id: &str) -> Result<PathBuf, StorageError> {
    let node_id = safe_segment(node_id)?;
    Ok(data_dir.join("runs").join(run_id.to_string()).join(node_id))
}

/// `{node_dir}/{socket_id}.{ext}`
pub fn single_output_path(node_dir: &Path, socket_id: &str, scalar: ScalarType) -> PathBuf {
    node_dir.join(format!("{socket_id}.{}", scalar.extension()))
}

/// `{node_dir}/{socket_id}/{index:04}.{ext}`
pub fn list_output_path(
    node_dir: &Path,
    socket_id: &str,
    scalar: ScalarType,
    index: usize,
) -> PathBuf {
    node_dir
        .join(socket_id)
        .join(format!("{index:04}.{}", scalar.extension()))
}

/// `{data_dir}/bundles/{bundle_id}`
pub fn bundle_dir(data_dir: &Path, bundle_id: Uuid) -> PathBuf {
    data_dir.join("bundles").join(bundle_id.to_string())
}

/// `{data_dir}/bundles/{bundle_id}/{file_id}.pdf`
pub fn bundle_file_path(data_dir: &Path, bundle_id: Uuid, file_id: Uuid) -> PathBuf {
    bundle_dir(data_dir, bundle_id).join(format!("{file_id}.pdf"))
}

/// Resolves `path` (symlinks and `..` included) and returns its canonical form only if it lies
/// inside `data_dir`. Blocking filesystem call.
pub fn ensure_within(data_dir: &Path, path: &Path) -> Result<PathBuf, StorageError> {
    let root = data_dir
        .canonicalize()
        .map_err(|_| StorageError::Unresolvable(data_dir.to_path_buf()))?;
    let resolved = path
        .canonicalize()
        .map_err(|_| StorageError::Unresolvable(path.to_path_buf()))?;
    if resolved.starts_with(&root) {
        Ok(resolved)
    } else {
        Err(StorageError::OutsideDataDir(resolved))
    }
}

/// `{node_dir}/_work`
pub fn work_dir(node_dir: &Path) -> PathBuf {
    node_dir.join("_work")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segment_accepts_and_rejects() {
        for ok in ["a", "node-1", "N_2", "0123456789", &"x".repeat(64)] {
            assert_eq!(safe_segment(ok), Ok(ok));
        }
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            "a b",
            ".",
            "한글",
            "a.b",
            &"x".repeat(65),
        ] {
            assert_eq!(
                safe_segment(bad),
                Err(StorageError::InvalidSegment(bad.to_owned())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn path_formats_follow_contract() {
        let run = Uuid::parse_str("3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f").unwrap();
        let dir = run_node_dir(Path::new("/data"), run, "n1").unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/data/runs/3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f/n1")
        );
        assert_eq!(
            single_output_path(&dir, "md", ScalarType::Md),
            dir.join("md.md")
        );
        assert_eq!(
            list_output_path(&dir, "images", ScalarType::Jpg, 3),
            dir.join("images/0003.jpg")
        );
        assert_eq!(
            list_output_path(&dir, "tables", ScalarType::Csv, 12345),
            dir.join("tables/12345.csv")
        );
        assert_eq!(work_dir(&dir), dir.join("_work"));
    }

    #[test]
    fn bundle_paths_follow_contract() {
        let bundle = Uuid::parse_str("3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f").unwrap();
        let file = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
        assert_eq!(
            bundle_dir(Path::new("/data"), bundle),
            PathBuf::from("/data/bundles/3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f")
        );
        assert_eq!(
            bundle_file_path(Path::new("/data"), bundle, file),
            PathBuf::from(
                "/data/bundles/3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f/11111111-2222-3333-4444-555555555555.pdf"
            )
        );
    }

    #[test]
    fn ensure_within_allows_inside_and_rejects_outside() {
        let root = std::env::temp_dir().join(format!("dochestra-storage-{}", Uuid::new_v4()));
        let inside = root.join("data");
        let outside = root.join("secret.txt");
        std::fs::create_dir_all(inside.join("sub")).unwrap();
        std::fs::write(inside.join("sub/ok.txt"), b"x").unwrap();
        std::fs::write(&outside, b"x").unwrap();

        let ok = ensure_within(&inside, &inside.join("sub/ok.txt")).unwrap();
        assert!(ok.ends_with("sub/ok.txt"));

        // `..` traversal that escapes the data dir.
        let traversal = inside.join("sub/../../secret.txt");
        assert!(matches!(
            ensure_within(&inside, &traversal),
            Err(StorageError::OutsideDataDir(_))
        ));
        // Sibling directory sharing the prefix is not "inside".
        let sibling = root.join("data-other");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("f"), b"x").unwrap();
        assert!(matches!(
            ensure_within(&inside, &sibling.join("f")),
            Err(StorageError::OutsideDataDir(_))
        ));
        // Symlink pointing outside.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, inside.join("link")).unwrap();
            assert!(matches!(
                ensure_within(&inside, &inside.join("link")),
                Err(StorageError::OutsideDataDir(_))
            ));
        }
        // Missing file.
        assert!(matches!(
            ensure_within(&inside, &inside.join("missing")),
            Err(StorageError::Unresolvable(_))
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn run_node_dir_rejects_unsafe_node_ids() {
        let run = Uuid::nil();
        assert!(run_node_dir(Path::new("/data"), run, "../etc").is_err());
        assert!(run_node_dir(Path::new("/data"), run, "").is_err());
    }
}
