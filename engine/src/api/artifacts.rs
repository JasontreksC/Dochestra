//! Run artifact download (CONTEXT §3.5, §4.1.3).

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use super::extract::ApiPath;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;
use crate::storage::ensure_within;
use crate::types::ScalarType;

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

#[derive(Debug, Deserialize)]
pub struct ArtifactQuery {
    /// Kept as text so a malformed value yields our standard 400 body.
    index: Option<String>,
}

fn artifact_not_found() -> ApiError {
    ApiError::NotFound("artifact not found".to_owned())
}

fn content_type(scalar: ScalarType) -> &'static str {
    match scalar {
        ScalarType::Pdf => "application/pdf",
        ScalarType::Md => "text/markdown; charset=utf-8",
        ScalarType::Json => "application/json",
        ScalarType::Csv => "text/csv; charset=utf-8",
        ScalarType::Jpg => "image/jpeg",
        ScalarType::Mp3 => "audio/mpeg",
    }
}

/// `Content-Disposition: inline` with an ASCII fallback name and an RFC 5987 `filename*`.
fn content_disposition(file_name: &str) -> String {
    let fallback: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .map(|c| if c == '"' || c == '\\' { '_' } else { c })
        .collect();
    let fallback = if fallback.trim().is_empty() {
        "download".to_owned()
    } else {
        fallback
    };

    let mut encoded = String::new();
    for byte in file_name.bytes() {
        let attr_char = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            );
        if attr_char {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("inline; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// `GET /api/runs/{runId}/artifacts/{nodeId}/{socketId}?index=n`
pub async fn download(
    State(state): State<AppState>,
    ApiPath((run_id, node_id, socket_id)): ApiPath<(Uuid, String, String)>,
    query: Result<Query<ArtifactQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(query) = query?;
    let index = match query.index.as_deref() {
        None => 0,
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| ApiError::BadRequest("index 는 0 이상의 정수여야 합니다".to_owned()))?,
    };

    let outputs = db::runs::get_node_outputs(&state.db, run_id, &node_id)
        .await?
        .ok_or_else(artifact_not_found)?;
    let artifact = outputs.get(&socket_id).ok_or_else(artifact_not_found)?;
    let path = artifact
        .paths
        .get(index)
        .ok_or_else(artifact_not_found)?
        .clone();

    // Stored paths are trusted only if they resolve inside DATA_DIR.
    let data_dir = state.config.data_dir.clone();
    let candidate = path.clone();
    let resolved = tokio::task::spawn_blocking(move || ensure_within(&data_dir, &candidate))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(|err| {
            tracing::warn!(%run_id, %node_id, %socket_id, error = %err, "artifact path rejected");
            artifact_not_found()
        })?;

    let file = tokio::fs::File::open(&resolved)
        .await
        .map_err(|_| artifact_not_found())?;
    let meta = file.metadata().await.map_err(|_| artifact_not_found())?;
    if !meta.is_file() {
        return Err(artifact_not_found());
    }

    let file_name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let headers = [
        (
            CONTENT_TYPE,
            content_type(artifact.data_type.scalar).to_owned(),
        ),
        (CONTENT_LENGTH, meta.len().to_string()),
        (CONTENT_DISPOSITION, content_disposition(&file_name)),
    ];
    Ok((headers, Body::from_stream(ReaderStream::new(file))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_name() {
        assert_eq!(
            content_disposition("md.md"),
            "inline; filename=\"md.md\"; filename*=UTF-8''md.md"
        );
    }

    #[test]
    fn korean_name_gets_fallback_and_percent_encoding() {
        assert_eq!(
            content_disposition("보고서 v1.pdf"),
            "inline; filename=\"___ v1.pdf\"; filename*=UTF-8''%EB%B3%B4%EA%B3%A0%EC%84%9C%20v1.pdf"
        );
    }

    #[test]
    fn quotes_and_control_chars_cannot_break_the_header() {
        let value = content_disposition("a\"b\\c\r\nd;e.pdf");
        assert!(
            value.starts_with("inline; filename=\"a_b_c__d;e.pdf\";"),
            "{value}"
        );
        assert!(!value.contains('\r') && !value.contains('\n'));
        assert!(
            value.contains("filename*=UTF-8''a%22b%5Cc%0D%0Ad%3Be.pdf"),
            "{value}"
        );
        assert!(axum::http::HeaderValue::from_str(&value).is_ok());
    }

    #[test]
    fn empty_fallback_uses_default_name() {
        assert!(content_disposition("").starts_with("inline; filename=\"download\";"));
    }

    #[test]
    fn content_types_follow_the_contract() {
        assert_eq!(content_type(ScalarType::Pdf), "application/pdf");
        assert_eq!(content_type(ScalarType::Md), "text/markdown; charset=utf-8");
        assert_eq!(content_type(ScalarType::Json), "application/json");
        assert_eq!(content_type(ScalarType::Csv), "text/csv; charset=utf-8");
        assert_eq!(content_type(ScalarType::Jpg), "image/jpeg");
        assert_eq!(content_type(ScalarType::Mp3), "audio/mpeg");
    }
}
