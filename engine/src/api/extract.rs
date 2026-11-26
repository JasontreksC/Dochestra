//! Request extractors whose rejections use the standard error body (CONTEXT §4.1.1).

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts};
use axum::http::StatusCode;

use crate::error::ApiError;

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        // Body-limit overflow is 413; every other body problem (syntax, schema, content type)
        // is reported as 400 so clients see a single error class for malformed requests.
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::PayloadTooLarge(rejection.body_text())
        } else {
            Self::BadRequest(rejection.body_text())
        }
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::BadRequest(rejection.body_text())
    }
}

/// `axum::Json` for request bodies, rejecting with [`ApiError`].
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

/// `axum::extract::Path`, rejecting with [`ApiError`].
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);
