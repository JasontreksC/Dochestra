//! `extract` node: PDF → Markdown, JSON, CSV tables and JPG images via the extractor service.

use std::path::{Path, PathBuf};

use crate::clients::extractor::{ExtractResponse, ExtractorClient, ExtractorError};
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::{StorageError, ensure_within};
use crate::types::{DataType, ScalarType};

pub async fn run(
    client: &ExtractorClient,
    data_dir: &Path,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let pdf = inputs
        .get("pdf")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 pdf 이(가) 없습니다".to_owned()))?;

    let response = client
        .extract(pdf, &ctx.out_dir)
        .await
        .map_err(|err| NodeError::Failed(describe(&err, client)))?;

    verify_paths(data_dir, &ctx.out_dir, &response).await?;

    let single = |scalar, path: PathBuf| Artifact {
        data_type: DataType::single(scalar),
        paths: vec![path],
    };
    let list = |scalar, paths: Vec<PathBuf>| Artifact {
        data_type: DataType::list_of(scalar),
        paths,
    };
    Ok(NodeOutputs::from([
        ("md".to_owned(), single(ScalarType::Md, response.md)),
        ("json".to_owned(), single(ScalarType::Json, response.json)),
        ("tables".to_owned(), list(ScalarType::Csv, response.tables)),
        ("images".to_owned(), list(ScalarType::Jpg, response.images)),
    ]))
}

fn describe(err: &ExtractorError, client: &ExtractorClient) -> String {
    match err {
        ExtractorError::Timeout => {
            format!("PDF 추출 시간 초과 ({}초)", client.timeout().as_secs())
        }
        ExtractorError::Connect(message) => format!("extractor 에 연결할 수 없습니다: {message}"),
        ExtractorError::Http { status, detail } => {
            format!("extractor 오류 (HTTP {status}): {detail}")
        }
        ExtractorError::Decode(message) => {
            format!("extractor 응답을 해석할 수 없습니다: {message}")
        }
    }
}

/// Every returned file must exist, lie inside `DATA_DIR` and inside this node's `out_dir`.
async fn verify_paths(
    data_dir: &Path,
    out_dir: &Path,
    response: &ExtractResponse,
) -> Result<(), NodeError> {
    let paths: Vec<PathBuf> = [&response.md, &response.json]
        .into_iter()
        .chain(&response.tables)
        .chain(&response.images)
        .cloned()
        .collect();
    let data_dir = data_dir.to_path_buf();
    let out_dir = out_dir.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let out_dir = ensure_within(&data_dir, &out_dir)
            .map_err(|e| NodeError::Other(anyhow::Error::new(e).context("node output dir")))?;
        for path in &paths {
            let resolved = match ensure_within(&data_dir, path) {
                Ok(resolved) => resolved,
                Err(StorageError::Unresolvable(_)) => {
                    return Err(NodeError::Failed(format!(
                        "extractor 가 반환한 파일이 없습니다: {}",
                        path.display()
                    )));
                }
                Err(_) => return Err(unexpected_path(path)),
            };
            if !resolved.starts_with(&out_dir) {
                return Err(unexpected_path(path));
            }
            if !resolved.is_file() {
                return Err(NodeError::Failed(format!(
                    "extractor 가 반환한 경로가 파일이 아닙니다: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| NodeError::Other(e.into()))?
}

fn unexpected_path(path: &Path) -> NodeError {
    NodeError::Failed(format!(
        "extractor 가 예상 밖 경로를 반환했습니다: {}",
        path.display()
    ))
}
