//! Claude (Anthropic Messages API) client over raw HTTP (CONTEXT §4.3).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use reqwest::header::RETRY_AFTER;
use serde_json::{Value, json};

const ANTHROPIC_VERSION: &str = "2026-10-21";
const ANTHROPIC_BETA: &str = "server-side-fallback-2026-07-01";
const MAX_TOKENS: u32 = 16_000;
const MAX_RETRIES: u32 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Upper bound for a server-supplied `retry-after`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
const MESSAGE_PREVIEW_CHARS: usize = 300;
/// Fixed allowance for JSON structure, `model`, `output_config`, … in the size estimate.
const REQUEST_OVERHEAD_BYTES: usize = 4 * 1024;

/// Upper bound for the estimated request body (the API itself rejects bodies above ~32 MB).
pub const MAX_REQUEST_BYTES: usize = 30 * 1024 * 1024;

/// One block of the user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserBlock {
    Text(String),
    ImageJpeg(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system: String,
    pub user_content: Vec<UserBlock>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LlmError {
    #[error("LLM_API_KEY 가 설정되지 않았습니다")]
    MissingApiKey,
    #[error("LLM 이 요청을 거절했습니다{}", category.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Refusal { category: Option<String> },
    #[error("LLM 출력이 최대 길이에 도달했습니다")]
    MaxTokens,
    #[error("문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다")]
    RequestTooLarge,
    #[error("LLM API 오류 (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    #[error("LLM API 에 연결할 수 없습니다: {0}")]
    Transport(String),
    #[error("LLM 응답을 해석할 수 없습니다: {0}")]
    Decode(String),
}

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    effort: String,
    timeout: Duration,
    backoff_base: Duration,
}

// Manual `Debug`: the API key must never show up in logs.
impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("effort", &self.effort)
            .finish_non_exhaustive()
    }
}

impl LlmClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
        effort: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.is_empty()),
            model: model.into(),
            effort: effort.into(),
            timeout: REQUEST_TIMEOUT,
            backoff_base: DEFAULT_BACKOFF_BASE,
        }
    }

    /// Base delay of the exponential retry backoff (1s by default; tests shrink it).
    #[must_use]
    pub fn with_backoff_base(mut self, base: Duration) -> Self {
        self.backoff_base = base;
        self
    }

    /// Per-request timeout (10 minutes by default).
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sends one message and returns the concatenated `text` blocks of the answer.
    ///
    /// 429/500/502/503/529 and connection errors are retried up to 3 times (exponential
    /// backoff, `retry-after` first); other 4xx fail immediately.
    pub async fn complete(&self, req: LlmRequest) -> Result<String, LlmError> {
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(LlmError::MissingApiKey);
        };
        if estimate_request_bytes(&req.system, &req.user_content) > MAX_REQUEST_BYTES {
            return Err(LlmError::RequestTooLarge);
        }
        let body = self.build_body(&req);
        let url = format!("{}/v1/messages", self.base_url);

        let mut attempt = 0u32;
        loop {
            let outcome = self.send_once(&url, api_key, &body).await;
            let (error, retry_after) = match outcome {
                Ok(text) => return Ok(text),
                Err(failure) => failure,
            };
            if !error.retryable || attempt >= MAX_RETRIES {
                return Err(self.redact_error(error.error, api_key));
            }
            let delay = retry_after.unwrap_or_else(|| self.backoff_base * 2u32.pow(attempt));
            tracing::warn!(
                attempt = attempt + 1,
                delay_ms = delay.as_millis(),
                "LLM request failed; retrying"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    fn build_body(&self, req: &LlmRequest) -> Value {
        // Images first (stable), then text.
        let images = req.user_content.iter().filter_map(|b| match b {
            UserBlock::ImageJpeg(bytes) => Some(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": STANDARD.encode(bytes),
                }
            })),
            UserBlock::Text(_) => None,
        });
        let texts = req.user_content.iter().filter_map(|b| match b {
            UserBlock::Text(text) => Some(json!({"type": "text", "text": text})),
            UserBlock::ImageJpeg(_) => None,
        });
        let content: Vec<Value> = images.chain(texts).collect();

        json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": req.system,
            "output_config": { "effort": self.effort },
            "fallbacks": "default",
            "messages": [{ "role": "user", "content": content }],
        })
    }

    async fn send_once(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> Result<String, (Failure, Option<Duration>)> {
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", ANTHROPIC_BETA)
            .json(body)
            .send()
            .await
            .map_err(|err| {
                let retryable = err.is_connect();
                (
                    Failure {
                        error: LlmError::Transport(err.to_string()),
                        retryable,
                    },
                    None,
                )
            })?;

        let status = response.status();
        let retry_after = parse_retry_after(&response);
        let text = response.text().await.map_err(|err| {
            (
                Failure {
                    error: LlmError::Transport(err.to_string()),
                    retryable: false,
                },
                None,
            )
        })?;

        if status.is_success() {
            return parse_success(&text).map_err(|error| {
                (
                    Failure {
                        error,
                        retryable: false,
                    },
                    None,
                )
            });
        }
        Err((
            Failure {
                error: LlmError::Api {
                    status: status.as_u16(),
                    message: error_message(&text),
                },
                retryable: is_retryable(status),
            },
            retry_after,
        ))
    }

    /// Defense in depth: scrub the key from any message before it can reach logs or users.
    fn redact_error(&self, error: LlmError, api_key: &str) -> LlmError {
        let scrub = |s: String| s.replace(api_key, "***");
        match error {
            LlmError::Api { status, message } => LlmError::Api {
                status,
                message: scrub(message),
            },
            LlmError::Transport(m) => LlmError::Transport(scrub(m)),
            LlmError::Decode(m) => LlmError::Decode(scrub(m)),
            other => other,
        }
    }
}

struct Failure {
    error: LlmError,
    retryable: bool,
}

/// Estimated size in bytes of the request body: UTF-8 text plus base64 image data plus a fixed
/// 4 KiB overhead. Slightly optimistic for texts that need much JSON escaping, which the gap
/// between [`MAX_REQUEST_BYTES`] and the API's own limit absorbs.
pub fn estimate_request_bytes(system: &str, blocks: &[UserBlock]) -> usize {
    let content: usize = blocks
        .iter()
        .map(|block| match block {
            UserBlock::Text(text) => text.len(),
            UserBlock::ImageJpeg(bytes) => base64_len(bytes.len()),
        })
        .sum();
    system.len() + content + REQUEST_OVERHEAD_BYTES
}

/// Length of the padded base64 encoding of `n` bytes: `4 * ceil(n / 3)`.
pub fn base64_len(n: usize) -> usize {
    4 * n.div_ceil(3)
}

fn is_retryable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 529)
}

fn parse_retry_after(response: &reqwest::Response) -> Option<Duration> {
    let seconds: f64 = response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(seconds).min(MAX_RETRY_AFTER))
}

/// `error.message` of an Anthropic error body, else the first 300 chars of the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(MESSAGE_PREVIEW_CHARS).collect())
}

fn parse_success(body: &str) -> Result<String, LlmError> {
    let value: Value = serde_json::from_str(body).map_err(|e| LlmError::Decode(e.to_string()))?;

    match value["stop_reason"].as_str() {
        Some("refusal") => {
            return Err(LlmError::Refusal {
                category: value["stop_details"]["category"]
                    .as_str()
                    .map(str::to_owned),
            });
        }
        Some("max_tokens") => return Err(LlmError::MaxTokens),
        _ => {}
    }

    let blocks = value["content"]
        .as_array()
        .ok_or_else(|| LlmError::Decode("content 배열이 없습니다".to_owned()))?;
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if texts.is_empty() {
        return Err(LlmError::Decode("text 블록이 없습니다".to_owned()));
    }
    Ok(texts.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_prefers_error_message_field() {
        assert_eq!(
            error_message(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad model"}}"#
            ),
            "bad model"
        );
        assert_eq!(error_message("upstream down"), "upstream down");
        assert_eq!(error_message(&"x".repeat(1000)).chars().count(), 300);
    }

    #[test]
    fn success_parsing_covers_stop_reasons_and_block_types() {
        let ok = r#"{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"A"},{"type":"fallback","x":1},{"type":"text","text":"B"}],"stop_reason":"end_turn"}"#;
        assert_eq!(parse_success(ok).unwrap(), "AB");
        assert_eq!(
            parse_success(
                r#"{"content":[],"stop_reason":"refusal","stop_details":{"category":"cbrn"}}"#
            ),
            Err(LlmError::Refusal {
                category: Some("cbrn".into())
            })
        );
        assert_eq!(
            parse_success(r#"{"content":[],"stop_reason":"refusal"}"#),
            Err(LlmError::Refusal { category: None })
        );
        assert_eq!(
            parse_success(
                r#"{"content":[{"type":"text","text":"cut"}],"stop_reason":"max_tokens"}"#
            ),
            Err(LlmError::MaxTokens)
        );
        assert!(matches!(
            parse_success(r#"{"content":[{"type":"thinking"}]}"#),
            Err(LlmError::Decode(_))
        ));
        assert!(matches!(
            parse_success("not json"),
            Err(LlmError::Decode(_))
        ));
    }

    #[test]
    fn error_messages_match_the_contract() {
        assert_eq!(
            LlmError::MissingApiKey.to_string(),
            "LLM_API_KEY 가 설정되지 않았습니다"
        );
        assert_eq!(
            LlmError::Refusal {
                category: Some("cbrn".into())
            }
            .to_string(),
            "LLM 이 요청을 거절했습니다 (cbrn)"
        );
        assert_eq!(
            LlmError::Refusal { category: None }.to_string(),
            "LLM 이 요청을 거절했습니다"
        );
        assert_eq!(
            LlmError::MaxTokens.to_string(),
            "LLM 출력이 최대 길이에 도달했습니다"
        );
    }

    #[test]
    fn retryable_statuses() {
        for code in [429, 500, 502, 503, 529] {
            assert!(is_retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
        for code in [400, 401, 403, 404, 413, 422] {
            assert!(!is_retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
    }

    #[test]
    fn debug_hides_the_key() {
        let client = LlmClient::new(
            reqwest::Client::new(),
            "http://x",
            Some("sk-secret".into()),
            "m",
            "high",
        );
        assert!(!format!("{client:?}").contains("sk-secret"));
    }
}
