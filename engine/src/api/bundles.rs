//! Bundle endpoints: PDF bundles stored on disk (CONTEXT §4.1.3).

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::State;
use axum::extract::multipart::{Multipart, MultipartError, MultipartRejection};
use axum::http::StatusCode;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::extract::{ApiJson, ApiPath};
use super::workflows::normalize_name;
use crate::db;
use crate::db::bundles::NewBundleFile;
use crate::error::ApiError;
use crate::state::AppState;
use crate::storage::{bundle_dir, bundle_file_path};
use crate::types::{Bundle, BundleFile, BundleSummary, CreateBundleRequest};

const PDF_MAGIC: &[u8; 5] = b"%PDF-";
const FALLBACK_FILE_NAME: &str = "file.pdf";

impl From<MultipartError> for ApiError {
    fn from(err: MultipartError) -> Self {
        if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::PayloadTooLarge("업로드 크기 한도를 초과했습니다".to_owned())
        } else {
            Self::BadRequest(err.body_text())
        }
    }
}

impl From<MultipartRejection> for ApiError {
    fn from(rejection: MultipartRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

fn bundle_not_found(id: Uuid) -> ApiError {
    ApiError::NotFound(format!("bundle {id} not found"))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<BundleSummary>>, ApiError> {
    Ok(Json(db::bundles::list(&state.db).await?))
}

pub async fn create(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateBundleRequest>,
) -> Result<(StatusCode, Json<Bundle>), ApiError> {
    let name = normalize_name(&req.name)?;
    let bundle = db::bundles::create(&state.db, name).await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

pub async fn get(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Bundle>, ApiError> {
    db::bundles::get(&state.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| bundle_not_found(id))
}

pub async fn delete(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !db::bundles::delete(&state.db, id).await? {
        return Err(bundle_not_found(id));
    }
    let dir = bundle_dir(&state.config.data_dir, id);
    if let Err(err) = tokio::fs::remove_dir_all(&dir).await
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %dir.display(), error = %err, "failed to remove bundle directory");
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_file(
    State(state): State<AppState>,
    ApiPath((bundle_id, file_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let stored = db::bundles::delete_file(&state.db, bundle_id, file_id)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(format!("file {file_id} not found in bundle {bundle_id}"))
        })?;
    let path = PathBuf::from(stored);
    if path.starts_with(&state.config.data_dir) {
        if let Err(err) = tokio::fs::remove_file(&path).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), error = %err, "failed to remove bundle file");
        }
    } else {
        tracing::warn!(path = %path.display(), "stored path outside data dir; not deleting");
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A file written to disk during an upload, not yet recorded in the DB.
struct Staged {
    id: Uuid,
    original_name: String,
    part_path: PathBuf,
    final_path: PathBuf,
    size: u64,
}

/// `POST /api/bundles/{id}/files` — multipart field `files` (repeatable), PDF only.
///
/// All-or-nothing: if any file is rejected or anything fails, every file written by this
/// request is removed and nothing is recorded.
pub async fn upload_files(
    State(state): State<AppState>,
    ApiPath(bundle_id): ApiPath<Uuid>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<Vec<BundleFile>>), ApiError> {
    let mut multipart = multipart?;
    if !db::bundles::exists(&state.db, bundle_id).await? {
        return Err(bundle_not_found(bundle_id));
    }

    let data_dir = state.config.data_dir.clone();
    let mut staged: Vec<Staged> = Vec::new();
    let outcome = receive_files(&data_dir, bundle_id, &mut multipart, &mut staged).await;
    if let Err(err) = outcome {
        discard(&staged).await;
        return Err(err);
    }
    if staged.is_empty() {
        return Err(ApiError::BadRequest(
            "multipart 필드 'files' 에 PDF 파일이 하나 이상 필요합니다".to_owned(),
        ));
    }

    for file in &staged {
        if let Err(err) = tokio::fs::rename(&file.part_path, &file.final_path).await {
            discard(&staged).await;
            return Err(ApiError::Internal(
                anyhow::Error::new(err).context("failed to finalize uploaded file"),
            ));
        }
    }

    let rows: Vec<NewBundleFile> = staged
        .iter()
        .map(|f| NewBundleFile {
            id: f.id,
            original_name: f.original_name.clone(),
            stored_path: f.final_path.to_string_lossy().into_owned(),
            size_bytes: i64::try_from(f.size).unwrap_or(i64::MAX),
        })
        .collect();
    match db::bundles::add_files(&state.db, bundle_id, &rows).await {
        Ok(Some(files)) => Ok((StatusCode::CREATED, Json(files))),
        Ok(None) => {
            discard(&staged).await;
            Err(bundle_not_found(bundle_id))
        }
        Err(err) => {
            discard(&staged).await;
            Err(err.into())
        }
    }
}

async fn receive_files(
    data_dir: &Path,
    bundle_id: Uuid,
    multipart: &mut Multipart,
    staged: &mut Vec<Staged>,
) -> Result<(), ApiError> {
    while let Some(mut field) = multipart.next_field().await? {
        if field.name() != Some("files") {
            continue;
        }
        let original_name = sanitize_file_name(field.file_name());
        if !original_name.to_ascii_lowercase().ends_with(".pdf") {
            return Err(ApiError::BadRequest(format!(
                "'{original_name}' 은(는) PDF 파일이 아닙니다 (확장자 .pdf 필요)"
            )));
        }

        let id = Uuid::new_v4();
        let final_path = bundle_file_path(data_dir, bundle_id, id);
        let part_path = final_path.with_extension("pdf.part");
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ApiError::Internal(anyhow::Error::new(e).context("create bundle dir"))
            })?;
        }
        let mut file = tokio::fs::File::create(&part_path)
            .await
            .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("create upload file")))?;
        staged.push(Staged {
            id,
            original_name: original_name.clone(),
            part_path,
            final_path,
            size: 0,
        });
        let entry = staged.len() - 1;

        let mut head: Vec<u8> = Vec::with_capacity(PDF_MAGIC.len());
        while let Some(chunk) = field.chunk().await? {
            if head.len() < PDF_MAGIC.len() {
                let take = (PDF_MAGIC.len() - head.len()).min(chunk.len());
                head.extend_from_slice(&chunk[..take]);
                if head.len() == PDF_MAGIC.len() && head != PDF_MAGIC {
                    return Err(not_a_pdf(&original_name));
                }
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("write upload")))?;
            staged[entry].size += chunk.len() as u64;
        }
        if head != PDF_MAGIC {
            return Err(not_a_pdf(&original_name));
        }
        file.flush()
            .await
            .map_err(|e| ApiError::Internal(anyhow::Error::new(e).context("flush upload")))?;
    }
    Ok(())
}

fn not_a_pdf(name: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "'{name}' 은(는) PDF 파일이 아닙니다 (%PDF- 시그니처 불일치)"
    ))
}

/// Keeps only the last path segment of a client-supplied name; empty becomes `file.pdf`.
fn sanitize_file_name(raw: Option<&str>) -> String {
    let last = raw
        .and_then(|name| name.rsplit(['/', '\\']).next())
        .map(str::trim)
        .unwrap_or_default();
    if last.is_empty() || last == "." || last == ".." {
        FALLBACK_FILE_NAME.to_owned()
    } else {
        last.to_owned()
    }
}

/// Best-effort removal of everything this request wrote.
async fn discard(staged: &[Staged]) {
    for file in staged {
        let _ = tokio::fs::remove_file(&file.part_path).await;
        let _ = tokio::fs::remove_file(&file.final_path).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_reduced_to_last_segment() {
        assert_eq!(sanitize_file_name(Some("a.pdf")), "a.pdf");
        assert_eq!(sanitize_file_name(Some("../../x.pdf")), "x.pdf");
        assert_eq!(sanitize_file_name(Some("C:\\dir\\y.PDF")), "y.PDF");
        assert_eq!(sanitize_file_name(Some("/abs/z.pdf")), "z.pdf");
        assert_eq!(sanitize_file_name(Some("dir/")), FALLBACK_FILE_NAME);
        assert_eq!(sanitize_file_name(Some("..")), FALLBACK_FILE_NAME);
        assert_eq!(sanitize_file_name(Some("   ")), FALLBACK_FILE_NAME);
        assert_eq!(sanitize_file_name(None), FALLBACK_FILE_NAME);
        assert_eq!(sanitize_file_name(Some("보고서.pdf")), "보고서.pdf");
    }
}
