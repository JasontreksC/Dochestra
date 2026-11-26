//! Retry policy and secret scrubbing shared by the external-API clients.

use std::future::Future;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::RETRY_AFTER;
use serde_json::Value;

/// Retries after the first attempt (so up to 4 requests in total).
pub const MAX_RETRIES: u32 = 3;
/// Base of the exponential backoff (1s, 2s, 4s) unless a client overrides it.
pub const DEFAULT_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Upper bound for a server-supplied `retry-after`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
const MESSAGE_PREVIEW_CHARS: usize = 300;

/// One failed attempt, as reported by a client's single-request function.
#[derive(Debug)]
pub struct Failure<E> {
    pub error: E,
    /// Whether the policy allows another attempt.
    pub retryable: bool,
    /// Server-requested delay (`retry-after`), preferred over the backoff.
    pub retry_after: Option<Duration>,
}

impl<E> Failure<E> {
    /// A failure that must not be retried.
    pub fn fatal(error: E) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: None,
        }
    }

    /// A failure of the HTTP exchange itself: only connection errors are retried (a timed-out
    /// request may still be running upstream, so repeating it could duplicate work).
    pub fn transport(err: &reqwest::Error, make: impl FnOnce(String) -> E) -> Self {
        Self {
            error: make(err.to_string()),
            retryable: err.is_connect(),
            retry_after: None,
        }
    }

    /// A non-success HTTP answer: 429/500/502/503 are retried, honouring `retry-after`.
    pub fn http(error: E, status: StatusCode, retry_after: Option<Duration>) -> Self {
        Self {
            error,
            retryable: is_retryable_status(status),
            retry_after,
        }
    }
}

/// Runs `attempt` until it succeeds, fails fatally, or the retries are used up. The error of the
/// last attempt is returned. `what` names the call in the retry log line.
pub async fn retry<T, E, F, Fut>(what: &str, backoff_base: Duration, mut attempt: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Failure<E>>>,
{
    let mut tries = 0u32;
    loop {
        let failure = match attempt().await {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        if !failure.retryable || tries >= MAX_RETRIES {
            return Err(failure.error);
        }
        let delay = failure
            .retry_after
            .unwrap_or_else(|| backoff_base * 2u32.pow(tries));
        tracing::warn!(
            what,
            attempt = tries + 1,
            delay_ms = delay.as_millis(),
            "request failed; retrying"
        );
        tokio::time::sleep(delay).await;
        tries += 1;
    }
}

pub fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503)
}

/// `retry-after` (seconds) of a response, capped at 60 seconds.
pub fn parse_retry_after(response: &reqwest::Response) -> Option<Duration> {
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

/// `error.message` of an API error body (OpenAI-compatible APIs share this shape), else the first
/// 300 characters of the body.
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(MESSAGE_PREVIEW_CHARS).collect())
}

/// Defense in depth: replaces every occurrence of `secret` with `***`.
pub fn scrub_secret(text: String, secret: &str) -> String {
    if secret.is_empty() {
        text
    } else {
        text.replace(secret, "***")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

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
        assert_eq!(error_message(&"가".repeat(400)).chars().count(), 300);
    }

    #[test]
    fn retryable_statuses() {
        for code in [429, 500, 502, 503] {
            assert!(
                is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{code}"
            );
        }
        for code in [400, 401, 403, 404, 413, 422, 529] {
            assert!(
                !is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{code}"
            );
        }
    }

    #[test]
    fn scrubbing_replaces_every_occurrence() {
        assert_eq!(
            scrub_secret("key sk-1 and sk-1".to_owned(), "sk-1"),
            "key *** and ***"
        );
        assert_eq!(scrub_secret("nothing".to_owned(), "sk-1"), "nothing");
        assert_eq!(scrub_secret("keep".to_owned(), ""), "keep");
    }

    fn failing(retryable: bool) -> Failure<&'static str> {
        Failure {
            error: "boom",
            retryable,
            retry_after: Some(Duration::ZERO),
        }
    }

    #[tokio::test]
    async fn retries_until_success_within_the_budget() {
        let calls = AtomicU32::new(0);
        let result: Result<u32, &str> = retry("test", Duration::ZERO, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move { if n < 3 { Err(failing(true)) } else { Ok(n) } }
        })
        .await;
        assert_eq!(result, Ok(3));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn gives_up_after_three_retries_and_returns_the_last_error() {
        let calls = AtomicU32::new(0);
        let result: Result<(), &str> = retry("test", Duration::ZERO, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(failing(true)) }
        })
        .await;
        assert_eq!(result, Err("boom"));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn fatal_failures_are_not_retried() {
        let calls = AtomicU32::new(0);
        let result: Result<(), &str> = retry("test", Duration::ZERO, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(Failure::fatal("fatal")) }
        })
        .await;
        assert_eq!(result, Err("fatal"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Runs on tokio's paused clock: sleeps complete instantly and `elapsed()` is the exact sum of
    /// the delays that were slept, so nothing here depends on how loaded the machine is.
    #[tokio::test(start_paused = true)]
    async fn backoff_doubles_and_retry_after_wins() {
        use tokio::time::Instant;

        let failing = |retry_after: Option<Duration>| async move {
            Err::<(), _>(Failure {
                error: "x",
                retryable: true,
                retry_after,
            })
        };

        // 10ms base: 10 + 20 + 40 ms between the four attempts.
        let start = Instant::now();
        let _: Result<(), &str> = retry("test", Duration::from_millis(10), || failing(None)).await;
        assert_eq!(start.elapsed(), Duration::from_millis(70));

        // A server-supplied delay replaces the backoff entirely: zero here (the base is 1s) ...
        let start = Instant::now();
        let _: Result<(), &str> = retry("test", Duration::from_secs(1), || {
            failing(Some(Duration::ZERO))
        })
        .await;
        assert_eq!(start.elapsed(), Duration::ZERO);

        // ... and a longer one is honored for each of the three retries, not capped by the backoff.
        let start = Instant::now();
        let _: Result<(), &str> = retry("test", Duration::from_millis(10), || {
            failing(Some(Duration::from_secs(2)))
        })
        .await;
        assert_eq!(start.elapsed(), Duration::from_secs(6));
    }
}
