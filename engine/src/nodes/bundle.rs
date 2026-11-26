//! `bundle` node: exposes the PDFs of a stored bundle (no copying, CONTEXT §3.5).

use std::path::PathBuf;

use sqlx::PgPool;
use uuid::Uuid;

use crate::db::bundles::{StoredFile, stored_files};
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeOutputs};
use crate::types::{DataType, ScalarType};

pub async fn run(db: &PgPool, ctx: &NodeContext) -> Result<NodeOutputs, NodeError> {
    let bundle_id = ctx
        .params
        .get("bundleId")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| NodeError::Failed("bundleId 가 올바른 UUID 가 아닙니다".to_owned()))?;

    let files = stored_files(db, bundle_id)
        .await
        .map_err(anyhow::Error::from)?
        .ok_or_else(|| NodeError::Failed(format!("번들 {bundle_id} 를 찾을 수 없습니다")))?;
    if files.is_empty() {
        return Err(NodeError::Failed("번들에 PDF 가 없습니다".to_owned()));
    }
    ensure_on_disk(&files).await?;

    let paths: Vec<PathBuf> = files
        .iter()
        .map(|f| PathBuf::from(&f.stored_path))
        .collect();
    Ok(NodeOutputs::from([(
        "files".to_owned(),
        Artifact {
            data_type: DataType::list_of(ScalarType::Pdf),
            paths,
        },
    )]))
}

async fn ensure_on_disk(files: &[StoredFile]) -> Result<(), NodeError> {
    for file in files {
        let present = tokio::fs::metadata(&file.stored_path)
            .await
            .is_ok_and(|m| m.is_file());
        if !present {
            return Err(NodeError::Failed(format!(
                "번들 파일이 디스크에 없습니다: {}",
                file.original_name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::bundles::{NewBundleFile, add_files, create};
    use crate::types::NodeKind;

    fn ctx(params: serde_json::Value) -> NodeContext {
        NodeContext {
            run_id: Uuid::new_v4(),
            node_id: "b".to_owned(),
            kind: NodeKind::Bundle,
            params,
            out_dir: std::env::temp_dir(),
        }
    }

    fn failure(result: Result<NodeOutputs, NodeError>) -> String {
        match result {
            Err(NodeError::Failed(message)) => message,
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("dochestra-bundle-node-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn add(db: &PgPool, bundle: Uuid, dir: &TempDir, name: &str, write: bool) -> PathBuf {
        let id = Uuid::new_v4();
        let path = dir.0.join(format!("{id}.pdf"));
        if write {
            std::fs::write(&path, b"%PDF-1.7").unwrap();
        }
        add_files(
            db,
            bundle,
            &[NewBundleFile {
                id,
                original_name: name.to_owned(),
                stored_path: path.to_string_lossy().into_owned(),
                size_bytes: 8,
            }],
        )
        .await
        .unwrap()
        .unwrap();
        path
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_bundle_fails(pool: PgPool) {
        let id = Uuid::new_v4();
        let message = failure(run(&pool, &ctx(json!({"bundleId": id}))).await);
        assert_eq!(message, format!("번들 {id} 를 찾을 수 없습니다"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn invalid_bundle_id_fails(pool: PgPool) {
        assert!(failure(run(&pool, &ctx(json!({"bundleId": "nope"}))).await).contains("UUID"));
        assert!(failure(run(&pool, &ctx(json!({}))).await).contains("UUID"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_bundle_fails(pool: PgPool) {
        let bundle = create(&pool, "empty").await.unwrap();
        let message = failure(run(&pool, &ctx(json!({"bundleId": bundle.id}))).await);
        assert_eq!(message, "번들에 PDF 가 없습니다");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn missing_disk_file_fails_with_original_name(pool: PgPool) {
        let dir = TempDir::new();
        let bundle = create(&pool, "b").await.unwrap();
        add(&pool, bundle.id, &dir, "present.pdf", true).await;
        add(&pool, bundle.id, &dir, "gone.pdf", false).await;
        let message = failure(run(&pool, &ctx(json!({"bundleId": bundle.id}))).await);
        assert_eq!(message, "번들 파일이 디스크에 없습니다: gone.pdf");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn outputs_all_files_in_position_order(pool: PgPool) {
        let dir = TempDir::new();
        let bundle = create(&pool, "b").await.unwrap();
        let first = add(&pool, bundle.id, &dir, "first.pdf", true).await;
        let second = add(&pool, bundle.id, &dir, "second.pdf", true).await;
        let third = add(&pool, bundle.id, &dir, "third.pdf", true).await;

        let outputs = run(&pool, &ctx(json!({"bundleId": bundle.id})))
            .await
            .unwrap();
        assert_eq!(outputs.len(), 1);
        let files = &outputs["files"];
        assert_eq!(files.data_type, DataType::list_of(ScalarType::Pdf));
        assert_eq!(files.paths, vec![first, second, third]);
    }
}
