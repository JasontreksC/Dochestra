//! OpenAI embeddings client (CONTEXT §4.3).

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::RETRY_AFTER;
use serde::Deserialize;
use serde_json::{Value, json};

/// Embedding dimension of `rag_chunks.embedding` (`vector(1536)`).
pub const EMBEDDING_DIMENSIONS: usize = 1536;
/// Maximum inputs per request.
pub const MAX_BATCH_ITEMS: usize = 96;
/// Maximum total characters per request.
pub const MAX_BATCH_CHARS: usize = 200_000;

const MAX_RETRIES: u32 = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_BACKOFF_BASE: Duration = Duration::from_secs(1);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
const MESSAGE_PREVIEW_CHARS: usize = 300;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbeddingError {
    #[error("EMBEDDING_API_KEY 가 설정되지 않았습니다")]
    MissingApiKey,
    #[error("임베딩 입력이 너무 깁니다 ({chars}자, 요청당 최대 200000자)")]
    InputTooLarge { chars: usize },
    #[error("임베딩 API 오류 (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    #[error("임베딩 API 에 연결할 수 없습니다: {0}")]
    Transport(String),
    #[error("임베딩 응답을 해석할 수 없습니다: {0}")]
    Decode(String),
    #[error("임베딩 차원이 {actual} 입니다 (기대 {expected})")]
    DimensionMismatch { expected: usize, actual: usize },
}

#[derive(Clone)]
pub struct EmbeddingClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    timeout: Duration,
    backoff_base: Duration,
}

// Manual `Debug`: the API key must never show up in logs.
impl std::fmt::Debug for EmbeddingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl EmbeddingClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.is_empty()),
            model: model.into(),
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

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Embeds `texts`, returning one 1536-dimensional vector per input, in input order.
    ///
    /// Inputs are sent in as few requests as possible (at most 96 inputs and 200,000
    /// characters each). 429/5xx and connection errors are retried up to 3 times.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(EmbeddingError::MissingApiKey);
        };
        let batches = plan_batches(texts)?;

        let mut vectors = Vec::with_capacity(texts.len());
        for range in batches {
            let embedded = self.embed_batch(api_key, &texts[range]).await?;
            vectors.extend(embedded);
        }
        Ok(vectors)
    }

    async fn embed_batch(
        &self,
        api_key: &str,
        batch: &[String],
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let body = json!({
            "model": self.model,
            "input": batch,
            "encoding_format": "float",
        });

        let mut attempt = 0u32;
        loop {
            let (error, retryable, retry_after) = match self.send_once(&url, api_key, &body).await {
                Ok(text) => return parse_embeddings(&text, batch.len()),
                Err(failure) => failure,
            };
            if !retryable || attempt >= MAX_RETRIES {
                return Err(redact(error, api_key));
            }
            let delay = retry_after.unwrap_or_else(|| self.backoff_base * 2u32.pow(attempt));
            tracing::warn!(
                attempt = attempt + 1,
                delay_ms = delay.as_millis(),
                "embedding request failed; retrying"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    /// One HTTP exchange; `Err` carries (error, retryable, retry-after).
    async fn send_once(
        &self,
        url: &str,
        api_key: &str,
        body: &Value,
    ) -> Result<String, (EmbeddingError, bool, Option<Duration>)> {
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .map_err(|err| {
                let retryable = err.is_connect();
                (EmbeddingError::Transport(err.to_string()), retryable, None)
            })?;

        let status = response.status();
        let retry_after = parse_retry_after(&response);
        let text = response
            .text()
            .await
            .map_err(|err| (EmbeddingError::Transport(err.to_string()), false, None))?;
        if status.is_success() {
            return Ok(text);
        }
        Err((
            EmbeddingError::Api {
                status: status.as_u16(),
                message: error_message(&text),
            },
            is_retryable(status),
            retry_after,
        ))
    }
}

/// Splits `texts` into consecutive batches of at most 96 inputs / 200,000 characters.
fn plan_batches(texts: &[String]) -> Result<Vec<std::ops::Range<usize>>, EmbeddingError> {
    let mut batches = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    for (i, text) in texts.iter().enumerate() {
        let len = text.chars().count();
        if len > MAX_BATCH_CHARS {
            return Err(EmbeddingError::InputTooLarge { chars: len });
        }
        let full = i - start >= MAX_BATCH_ITEMS || chars + len > MAX_BATCH_CHARS;
        if i > start && full {
            batches.push(start..i);
            start = i;
            chars = 0;
        }
        chars += len;
    }
    if start < texts.len() {
        batches.push(start..texts.len());
    }
    Ok(batches)
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

/// Orders the response by `index`, and checks count, index coverage and dimensions.
fn parse_embeddings(body: &str, expected: usize) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    let mut parsed: EmbeddingsResponse =
        serde_json::from_str(body).map_err(|e| EmbeddingError::Decode(e.to_string()))?;
    if parsed.data.len() != expected {
        return Err(EmbeddingError::Decode(format!(
            "입력 {expected}개에 대해 {}개의 임베딩이 반환되었습니다",
            parsed.data.len()
        )));
    }
    parsed.data.sort_by_key(|item| item.index);
    for (position, item) in parsed.data.iter().enumerate() {
        if item.index != position {
            return Err(EmbeddingError::Decode(
                "index 값이 0..n 을 이루지 않습니다".to_owned(),
            ));
        }
        if item.embedding.len() != EMBEDDING_DIMENSIONS {
            return Err(EmbeddingError::DimensionMismatch {
                expected: EMBEDDING_DIMENSIONS,
                actual: item.embedding.len(),
            });
        }
    }
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
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

/// `error.message` of an OpenAI error body, else the first 300 chars of the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(MESSAGE_PREVIEW_CHARS).collect())
}

/// Defense in depth: scrub the key from any message before it can reach logs or users.
fn redact(error: EmbeddingError, api_key: &str) -> EmbeddingError {
    let scrub = |s: String| s.replace(api_key, "***");
    match error {
        EmbeddingError::Api { status, message } => EmbeddingError::Api {
            status,
            message: scrub(message),
        },
        EmbeddingError::Transport(m) => EmbeddingError::Transport(scrub(m)),
        EmbeddingError::Decode(m) => EmbeddingError::Decode(scrub(m)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lens: &[usize]) -> Vec<String> {
        lens.iter().map(|n| "a".repeat(*n)).collect()
    }

    fn spans(pairs: &[(usize, usize)]) -> Vec<std::ops::Range<usize>> {
        pairs.iter().map(|(start, end)| *start..*end).collect()
    }

    #[test]
    fn batches_split_on_item_count() {
        let all = texts(&vec![1; 200]);
        let batches = plan_batches(&all).unwrap();
        assert_eq!(batches, spans(&[(0, 96), (96, 192), (192, 200)]));
        assert_eq!(
            plan_batches(&texts(&vec![1; 96])).unwrap(),
            spans(&[(0, 96)])
        );
        assert!(plan_batches(&[]).unwrap().is_empty());
    }

    #[test]
    fn batches_split_on_total_characters() {
        // 90k + 90k fit (180k); the third would make 270k.
        assert_eq!(
            plan_batches(&texts(&[90_000, 90_000, 90_000])).unwrap(),
            spans(&[(0, 2), (2, 3)])
        );
        // Exactly 200,000 chars in total is allowed.
        assert_eq!(
            plan_batches(&texts(&[100_000, 100_000])).unwrap(),
            spans(&[(0, 2)])
        );
        assert_eq!(
            plan_batches(&texts(&[100_000, 100_001])).unwrap(),
            spans(&[(0, 1), (1, 2)])
        );
        // Characters, not bytes.
        let korean = vec!["가".repeat(100_000), "가".repeat(100_000)];
        assert_eq!(plan_batches(&korean).unwrap(), spans(&[(0, 2)]));
    }

    #[test]
    fn a_single_oversized_input_is_rejected() {
        assert_eq!(
            plan_batches(&texts(&[1, 200_001])),
            Err(EmbeddingError::InputTooLarge { chars: 200_001 })
        );
        assert_eq!(plan_batches(&texts(&[200_000])).unwrap(), spans(&[(0, 1)]));
    }

    fn response(items: &[(usize, usize)]) -> String {
        let data: Vec<Value> = items
            .iter()
            .map(|(index, dims)| json!({"object": "embedding", "index": index, "embedding": vec![0.5f32; *dims]}))
            .collect();
        json!({"object": "list", "data": data}).to_string()
    }

    #[test]
    fn responses_are_ordered_by_index() {
        let body = json!({"data": [
            {"index": 1, "embedding": vec![1.0f32; EMBEDDING_DIMENSIONS]},
            {"index": 0, "embedding": vec![0.0f32; EMBEDDING_DIMENSIONS]},
        ]})
        .to_string();
        let out = parse_embeddings(&body, 2).unwrap();
        assert_eq!(out[0][0], 0.0);
        assert_eq!(out[1][0], 1.0);
    }

    #[test]
    fn malformed_responses_are_rejected() {
        let d = EMBEDDING_DIMENSIONS;
        assert_eq!(
            parse_embeddings(&response(&[(0, 3)]), 1),
            Err(EmbeddingError::DimensionMismatch {
                expected: d,
                actual: 3
            })
        );
        assert!(matches!(
            parse_embeddings(&response(&[(0, d)]), 2),
            Err(EmbeddingError::Decode(_))
        ));
        assert!(matches!(
            parse_embeddings(&response(&[(0, d), (0, d)]), 2),
            Err(EmbeddingError::Decode(_))
        ));
        assert!(matches!(
            parse_embeddings(&response(&[(1, d)]), 1),
            Err(EmbeddingError::Decode(_))
        ));
        assert!(matches!(
            parse_embeddings("nope", 1),
            Err(EmbeddingError::Decode(_))
        ));
        assert_eq!(
            EmbeddingError::DimensionMismatch {
                expected: d,
                actual: 3
            }
            .to_string(),
            "임베딩 차원이 3 입니다 (기대 1536)"
        );
    }

    #[test]
    fn error_messages_and_debug() {
        assert_eq!(
            error_message(r#"{"error":{"message":"Incorrect API key"}}"#),
            "Incorrect API key"
        );
        assert_eq!(error_message("gateway down"), "gateway down");
        assert_eq!(
            EmbeddingError::MissingApiKey.to_string(),
            "EMBEDDING_API_KEY 가 설정되지 않았습니다"
        );
        let client = EmbeddingClient::new(
            reqwest::Client::new(),
            "http://x",
            Some("sk-secret".into()),
            "m",
        );
        assert!(!format!("{client:?}").contains("sk-secret"));
    }
}
