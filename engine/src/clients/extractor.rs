//! HTTP client for the extractor service (CONTEXT §4.2).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const DETAIL_PREVIEW_CHARS: usize = 300;

/// Response of `POST /extract`; keys are the extract node's output socket ids.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExtractResponse {
    pub md: PathBuf,
    pub json: PathBuf,
    pub tables: Vec<PathBuf>,
    pub images: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtractorError {
    #[error("request timed out")]
    Timeout,
    #[error("cannot reach extractor: {0}")]
    Connect(String),
    #[error("extractor returned HTTP {status}: {detail}")]
    Http { status: u16, detail: String },
    #[error("cannot decode extractor response: {0}")]
    Decode(String),
}

#[derive(Debug, Clone)]
pub struct ExtractorClient {
    http: reqwest::Client,
    base_url: String,
    timeout: Duration,
}

impl ExtractorClient {
    pub fn new(http: reqwest::Client, base_url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            timeout,
        }
    }

    /// Per-request timeout of `extract`.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// `GET /health`: `Ok(())` only on HTTP 200.
    pub async fn health(&self) -> Result<(), ExtractorError> {
        let resp = self
            .http
            .get(format!("{}/health", self.base_url))
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .map_err(map_transport_error)?;
        if resp.status().is_success() {
            return Ok(());
        }
        Err(http_error(resp).await)
    }

    /// `POST /extract` for one PDF; the extractor writes its results below `output_dir`.
    pub async fn extract(
        &self,
        pdf: &Path,
        output_dir: &Path,
    ) -> Result<ExtractResponse, ExtractorError> {
        let resp = self
            .http
            .post(format!("{}/extract", self.base_url))
            .timeout(self.timeout)
            .json(&json!({
                "pdf_path": pdf.to_string_lossy(),
                "output_dir": output_dir.to_string_lossy(),
            }))
            .send()
            .await
            .map_err(map_transport_error)?;
        if !resp.status().is_success() {
            return Err(http_error(resp).await);
        }
        resp.json::<ExtractResponse>().await.map_err(|err| {
            if err.is_timeout() {
                ExtractorError::Timeout
            } else {
                ExtractorError::Decode(err.to_string())
            }
        })
    }
}

fn map_transport_error(err: reqwest::Error) -> ExtractorError {
    if err.is_timeout() {
        ExtractorError::Timeout
    } else {
        ExtractorError::Connect(err.to_string())
    }
}

async fn http_error(resp: reqwest::Response) -> ExtractorError {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    ExtractorError::Http {
        status,
        detail: parse_detail(&body),
    }
}

/// FastAPI error body → readable detail: a string `detail` as-is, any other `detail` value
/// (e.g. the 422 validation array) as JSON text, otherwise the first 300 chars of the body.
fn parse_detail(body: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(map)) => match map.get("detail") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => preview(body),
        },
        _ => preview(body),
    }
}

fn preview(body: &str) -> String {
    body.chars().take(DETAIL_PREVIEW_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_detail_is_used_verbatim() {
        assert_eq!(
            parse_detail(r#"{"detail":"pdf not found: /data/x.pdf"}"#),
            "pdf not found: /data/x.pdf"
        );
    }

    #[test]
    fn non_string_detail_is_serialized() {
        let body = r#"{"detail":[{"loc":["body","pdf_path"],"msg":"Field required"}]}"#;
        assert_eq!(
            parse_detail(body),
            r#"[{"loc":["body","pdf_path"],"msg":"Field required"}]"#
        );
    }

    #[test]
    fn non_json_or_detail_less_bodies_are_truncated_previews() {
        assert_eq!(parse_detail("Bad Gateway"), "Bad Gateway");
        let long = "x".repeat(1000);
        assert_eq!(parse_detail(&long).chars().count(), 300);
        assert_eq!(
            parse_detail(r#"{"message":"nope"}"#),
            r#"{"message":"nope"}"#
        );
        let korean = "가".repeat(400);
        assert_eq!(parse_detail(&korean).chars().count(), 300);
        assert_eq!(parse_detail(""), "");
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let client = ExtractorClient::new(
            reqwest::Client::new(),
            "http://x:8000/",
            Duration::from_secs(1),
        );
        assert_eq!(client.base_url, "http://x:8000");
        assert_eq!(client.timeout(), Duration::from_secs(1));
    }
}
