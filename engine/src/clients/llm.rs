//! LLM client for OpenRouter's OpenAI-compatible `chat/completions` API (CONTEXT §4.3).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::common::{
    DEFAULT_BACKOFF_BASE, Failure, error_message, parse_retry_after, retry, scrub_secret,
};

const MAX_TOKENS: u32 = 16_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// Fixed allowance for JSON structure, `model`, `reasoning`, … in the size estimate.
const REQUEST_OVERHEAD_BYTES: usize = 4 * 1024;

/// Upper bound for the estimated request body (the providers' own limits are ~32 MB).
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
    #[error("OPENROUTER_API_KEY 가 설정되지 않았습니다")]
    MissingApiKey,
    #[error("LLM 이 요청을 거절했습니다")]
    Refusal,
    #[error("LLM 출력이 최대 길이에 도달했습니다")]
    MaxTokens,
    #[error("LLM 응답이 비어 있습니다")]
    Empty,
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

    /// Sends one chat request and returns the assistant's text.
    ///
    /// 429/500/502/503 and connection errors are retried up to 3 times (exponential backoff,
    /// `retry-after` first); other 4xx fail immediately.
    pub async fn complete(&self, req: LlmRequest) -> Result<String, LlmError> {
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(LlmError::MissingApiKey);
        };
        if estimate_request_bytes(&req.system, &req.user_content) > MAX_REQUEST_BYTES {
            return Err(LlmError::RequestTooLarge);
        }
        let body = self.build_body(&req);
        let url = format!("{}/chat/completions", self.base_url);

        retry("LLM", self.backoff_base, || {
            self.send_once(&url, api_key, &body)
        })
        .await
        .map_err(|error| self.redact_error(error, api_key))
    }

    fn build_body(&self, req: &LlmRequest) -> Value {
        // Images first (stable), then text.
        let images = req.user_content.iter().filter_map(|b| match b {
            UserBlock::ImageJpeg(bytes) => Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)) },
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
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": content },
            ],
            "max_tokens": MAX_TOKENS,
            "reasoning": { "effort": self.effort },
        })
    }

    async fn send_once(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> Result<String, Failure<LlmError>> {
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .bearer_auth(api_key)
            .header("X-Title", "Dochestra")
            .json(body)
            .send()
            .await
            .map_err(|err| Failure::transport(&err, LlmError::Transport))?;

        let status = response.status();
        let retry_after = parse_retry_after(&response);
        let text = response
            .text()
            .await
            .map_err(|err| Failure::fatal(LlmError::Transport(err.to_string())))?;

        if status.is_success() {
            return parse_success(&text);
        }
        Err(Failure::http(
            LlmError::Api {
                status: status.as_u16(),
                message: error_message(&text),
            },
            status,
            retry_after,
        ))
    }

    /// Defense in depth: scrub the key from any message before it can reach logs or users.
    fn redact_error(&self, error: LlmError, api_key: &str) -> LlmError {
        let scrub = |s: String| scrub_secret(s, api_key);
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

/// Estimated size in bytes of the request body: UTF-8 text plus base64 image data plus a fixed
/// 4 KiB overhead. Slightly optimistic for texts that need much JSON escaping, which the gap
/// between [`MAX_REQUEST_BYTES`] and the providers' own limits absorbs.
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

/// Interprets a 200 response: a body-level `error` (OpenRouter can report upstream failures this
/// way), `message.refusal`, `finish_reason: length`, empty content, or the answer text.
fn parse_success(body: &str) -> Result<String, Failure<LlmError>> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| Failure::fatal(LlmError::Decode(e.to_string())))?;

    if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
        let code = error["code"].as_u64().and_then(|c| u16::try_from(c).ok());
        let status = code
            .and_then(|c| reqwest::StatusCode::from_u16(c).ok())
            .unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
        let message = error["message"]
            .as_str()
            .map_or_else(|| error.to_string(), str::to_owned);
        return Err(Failure::http(
            LlmError::Api {
                status: status.as_u16(),
                message,
            },
            status,
            None,
        ));
    }

    let choice = value["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or_else(|| Failure::fatal(LlmError::Decode("choices 가 없습니다".to_owned())))?;
    let message = &choice["message"];

    if message["refusal"]
        .as_str()
        .is_some_and(|r| !r.trim().is_empty())
    {
        return Err(Failure::fatal(LlmError::Refusal));
    }
    if choice["finish_reason"] == "length" {
        return Err(Failure::fatal(LlmError::MaxTokens));
    }

    let text = match &message["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        return Err(Failure::fatal(LlmError::Empty));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(body: Value) -> Result<String, LlmError> {
        parse_success(&body.to_string()).map_err(|f| f.error)
    }

    #[test]
    fn answers_are_extracted_from_string_and_array_content() {
        let ok = json!({"choices": [{"message": {"role": "assistant", "content": "Hello"}, "finish_reason": "stop"}]});
        assert_eq!(parsed(ok).unwrap(), "Hello");

        let parts = json!({"choices": [{"message": {"content": [
            {"type": "reasoning", "text": "ignored"},
            {"type": "text", "text": "A"},
            {"type": "text", "text": "B"}
        ]}, "finish_reason": "stop"}]});
        assert_eq!(parsed(parts).unwrap(), "AB");
    }

    #[test]
    fn finish_reasons_refusals_and_empty_answers_map_to_errors() {
        assert_eq!(
            parsed(
                json!({"choices": [{"message": {"content": "cut off"}, "finish_reason": "length"}]})
            ),
            Err(LlmError::MaxTokens)
        );
        // Reasoning models can spend the whole budget on thinking: content null + length.
        assert_eq!(
            parsed(json!({"choices": [{"message": {"content": null}, "finish_reason": "length"}]})),
            Err(LlmError::MaxTokens)
        );
        assert_eq!(
            parsed(
                json!({"choices": [{"message": {"content": null, "refusal": "I can't help with that"}, "finish_reason": "stop"}]})
            ),
            Err(LlmError::Refusal)
        );
        for empty in [json!(null), json!(""), json!("  \n"), json!([])] {
            assert_eq!(
                parsed(
                    json!({"choices": [{"message": {"content": empty}, "finish_reason": "stop"}]})
                ),
                Err(LlmError::Empty)
            );
        }
        // An empty refusal string is not a refusal.
        assert_eq!(
            parsed(json!({"choices": [{"message": {"content": "fine", "refusal": ""}, "finish_reason": "stop"}]})).unwrap(),
            "fine"
        );
        assert!(matches!(
            parsed(json!({"choices": []})),
            Err(LlmError::Decode(_))
        ));
        assert!(matches!(
            parsed(json!({"id": "x"})),
            Err(LlmError::Decode(_))
        ));
        assert!(matches!(
            parse_success("not json").map_err(|f| f.error),
            Err(LlmError::Decode(_))
        ));
    }

    #[test]
    fn a_body_level_error_becomes_an_api_error_with_its_code() {
        let err = parse_success(
            &json!({"error": {"message": "Provider returned error", "code": 429}}).to_string(),
        )
        .unwrap_err();
        assert!(err.retryable, "429 inside a 200 body is still retried");
        assert_eq!(
            err.error,
            LlmError::Api {
                status: 429,
                message: "Provider returned error".into()
            }
        );
        let err =
            parse_success(&json!({"error": {"message": "bad input", "code": 400}}).to_string())
                .unwrap_err();
        assert!(!err.retryable);
        // Without a usable code it is treated as a (retryable) gateway failure.
        let err = parse_success(&json!({"error": {"message": "boom"}}).to_string()).unwrap_err();
        assert_eq!(
            err.error,
            LlmError::Api {
                status: 502,
                message: "boom".into()
            }
        );
    }

    #[test]
    fn error_messages_match_the_contract() {
        assert_eq!(
            LlmError::MissingApiKey.to_string(),
            "OPENROUTER_API_KEY 가 설정되지 않았습니다"
        );
        assert_eq!(LlmError::Refusal.to_string(), "LLM 이 요청을 거절했습니다");
        assert_eq!(
            LlmError::MaxTokens.to_string(),
            "LLM 출력이 최대 길이에 도달했습니다"
        );
        assert_eq!(LlmError::Empty.to_string(), "LLM 응답이 비어 있습니다");
        assert_eq!(
            LlmError::RequestTooLarge.to_string(),
            "문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다"
        );
    }

    #[test]
    fn base64_length_matches_the_encoder() {
        for n in [0usize, 1, 2, 3, 4, 5, 6, 100, 1023, 1024] {
            let encoded = STANDARD.encode(vec![7u8; n]);
            assert_eq!(base64_len(n), encoded.len(), "n = {n}");
        }
    }

    #[test]
    fn request_size_estimate() {
        let blocks = [
            UserBlock::Text("abc".to_owned()),
            UserBlock::Text("가".to_owned()), // 3 UTF-8 bytes
            UserBlock::ImageJpeg(vec![0; 3]), // 4 base64 chars
            UserBlock::ImageJpeg(vec![0; 4]), // 8 base64 chars (padded)
        ];
        assert_eq!(
            estimate_request_bytes("sys", &blocks),
            3 + 3 + 3 + 4 + 8 + 4096
        );
        assert_eq!(estimate_request_bytes("", &[]), 4096);
        assert_eq!(MAX_REQUEST_BYTES, 31_457_280);
    }

    #[test]
    fn debug_hides_the_key() {
        let client = LlmClient::new(
            reqwest::Client::new(),
            "http://x",
            Some("sk-secret".into()),
            "m",
            "medium",
        );
        assert!(!format!("{client:?}").contains("sk-secret"));
    }
}
