//! API error type and HTTP mapping (CONTEXT §4.1.1).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::types::{ApiErrorBody, ApiErrorDetail, ValidationIssue};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
    #[error("validation failed: {message}")]
    Validation {
        message: String,
        issues: Vec<ValidationIssue>,
    },
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl ApiError {
    fn into_parts(self) -> (StatusCode, ApiErrorDetail) {
        let detail =
            |code: &str, message: String, issues: Option<Vec<ValidationIssue>>| ApiErrorDetail {
                code: code.to_owned(),
                message,
                issues,
            };
        match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, detail("BAD_REQUEST", m, None)),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, detail("NOT_FOUND", m, None)),
            Self::Conflict(m) => (StatusCode::CONFLICT, detail("CONFLICT", m, None)),
            Self::PayloadTooLarge(m) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                detail("PAYLOAD_TOO_LARGE", m, None),
            ),
            Self::Validation { message, issues } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                detail("VALIDATION_FAILED", message, Some(issues)),
            ),
            Self::Upstream(m) => (StatusCode::BAD_GATEWAY, detail("UPSTREAM", m, None)),
            Self::Internal(cause) => {
                tracing::error!(error = ?cause, "internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    detail("INTERNAL", "internal server error".to_owned(), None),
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = self.into_parts();
        (status, Json(ApiErrorBody { error })).into_response()
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use serde_json::{Value, json};

    use super::*;

    async fn render(err: ApiError) -> (StatusCode, Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn check(err: ApiError, status: StatusCode, code: &str, message: &str) {
        let (s, body) = render(err).await;
        assert_eq!(s, status);
        assert_eq!(
            body,
            json!({ "error": { "code": code, "message": message } })
        );
    }

    #[tokio::test]
    async fn bad_request() {
        check(
            ApiError::BadRequest("bad".into()),
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "bad",
        )
        .await;
    }

    #[tokio::test]
    async fn not_found() {
        check(
            ApiError::NotFound("nf".into()),
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "nf",
        )
        .await;
    }

    #[tokio::test]
    async fn conflict() {
        check(
            ApiError::Conflict("c".into()),
            StatusCode::CONFLICT,
            "CONFLICT",
            "c",
        )
        .await;
    }

    #[tokio::test]
    async fn payload_too_large() {
        check(
            ApiError::PayloadTooLarge("big".into()),
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "big",
        )
        .await;
    }

    #[tokio::test]
    async fn validation_includes_issues() {
        let (s, body) = render(ApiError::Validation {
            message: "invalid".into(),
            issues: vec![ValidationIssue::NoOutputNode],
        })
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body,
            json!({"error": {
                "code": "VALIDATION_FAILED",
                "message": "invalid",
                "issues": [{"code": "NO_OUTPUT_NODE"}]
            }})
        );
    }

    #[tokio::test]
    async fn upstream() {
        check(
            ApiError::Upstream("up".into()),
            StatusCode::BAD_GATEWAY,
            "UPSTREAM",
            "up",
        )
        .await;
    }

    #[tokio::test]
    async fn internal_hides_cause() {
        check(
            ApiError::Internal(anyhow::anyhow!("secret db password leaked")),
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            "internal server error",
        )
        .await;
    }
}
