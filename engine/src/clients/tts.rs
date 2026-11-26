//! OpenAI text-to-speech client (CONTEXT §4.3).

use std::time::Duration;

use serde_json::json;

use super::common::{
    DEFAULT_BACKOFF_BASE, Failure, error_message, parse_retry_after, retry, scrub_secret,
};

/// Maximum characters per synthesis request (the API limit for `input`).
pub const MAX_INPUT_CHARS: usize = 4000;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TtsError {
    #[error("TTS_API_KEY 가 설정되지 않았습니다")]
    MissingApiKey,
    #[error("TTS API 오류 (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    #[error("TTS API 에 연결할 수 없습니다: {0}")]
    Transport(String),
    #[error("TTS 응답을 해석할 수 없습니다: {0}")]
    Decode(String),
}

#[derive(Clone)]
pub struct TtsClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    voice: String,
    timeout: Duration,
    backoff_base: Duration,
}

// Manual `Debug`: the API key must never show up in logs.
impl std::fmt::Debug for TtsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("voice", &self.voice)
            .finish_non_exhaustive()
    }
}

impl TtsClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
        voice: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|k| !k.is_empty()),
            model: model.into(),
            voice: voice.into(),
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

    /// Synthesizes `text` into one MP3.
    ///
    /// Text over 4000 characters is split at sentence boundaries, synthesized chunk by chunk in
    /// order, and the MP3 bytes are concatenated in the same order. Blank text yields no audio
    /// (and needs no key).
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let chunks = split_text(text, MAX_INPUT_CHARS);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(TtsError::MissingApiKey);
        };

        let mut audio = Vec::new();
        for chunk in &chunks {
            audio.extend(self.synthesize_chunk(api_key, chunk).await?);
        }
        Ok(audio)
    }

    async fn synthesize_chunk(&self, api_key: &str, input: &str) -> Result<Vec<u8>, TtsError> {
        let url = format!("{}/v1/audio/speech", self.base_url);
        let body = json!({
            "model": self.model,
            "input": input,
            "voice": self.voice,
            "response_format": "mp3",
        });
        retry("TTS", self.backoff_base, || {
            self.send_once(&url, api_key, &body)
        })
        .await
        .map_err(|error| redact(error, api_key))
    }

    async fn send_once(
        &self,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
    ) -> Result<Vec<u8>, Failure<TtsError>> {
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .map_err(|err| Failure::transport(&err, TtsError::Transport))?;

        let status = response.status();
        let retry_after = parse_retry_after(&response);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| Failure::fatal(TtsError::Transport(err.to_string())))?;

        if !status.is_success() {
            return Err(Failure::http(
                TtsError::Api {
                    status: status.as_u16(),
                    message: error_message(&String::from_utf8_lossy(&bytes)),
                },
                status,
                retry_after,
            ));
        }
        if !content_type.starts_with("audio/") {
            return Err(Failure::fatal(TtsError::Decode(format!(
                "오디오가 아닌 응답입니다 (Content-Type: {})",
                if content_type.is_empty() {
                    "없음"
                } else {
                    &content_type
                }
            ))));
        }
        if bytes.is_empty() {
            return Err(Failure::fatal(TtsError::Decode(
                "응답 본문이 비어 있습니다".to_owned(),
            )));
        }
        Ok(bytes.to_vec())
    }
}

/// Defense in depth: scrub the key from any message before it can reach logs or users.
fn redact(error: TtsError, api_key: &str) -> TtsError {
    let scrub = |s: String| scrub_secret(s, api_key);
    match error {
        TtsError::Api { status, message } => TtsError::Api {
            status,
            message: scrub(message),
        },
        TtsError::Transport(m) => TtsError::Transport(scrub(m)),
        TtsError::Decode(m) => TtsError::Decode(scrub(m)),
        other => other,
    }
}

/// Splits `text` into chunks of at most `max_chars` characters at sentence boundaries.
///
/// Sentences end after `.`/`!`/`?` followed by whitespace or the end of the text, after `。`,
/// `！`, `？`, and after a newline. Sentences are packed greedily in order; a single sentence
/// longer than the limit is hard-split every `max_chars` characters. Chunks are trimmed and
/// blank ones dropped, so no request ever carries empty input.
pub fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    let mut push_sentence = |sentence: String| {
        let len = sentence.chars().count();
        if len > max_chars {
            // Close the current chunk, then emit the oversized sentence in hard pieces.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let chars: Vec<char> = sentence.chars().collect();
            let mut pieces: Vec<String> = chars
                .chunks(max_chars)
                .map(|c| c.iter().collect())
                .collect();
            // Keep the (possibly short) last piece open so following sentences can join it.
            if let Some(last) = pieces.pop() {
                chunks.extend(pieces);
                current_len = last.chars().count();
                current = last;
            }
        } else if current_len + len > max_chars {
            chunks.push(std::mem::take(&mut current));
            current = sentence;
            current_len = len;
        } else {
            current.push_str(&sentence);
            current_len += len;
        }
    };

    let mut sentence = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        sentence.push(c);
        let ends = match c {
            '\n' | '。' | '！' | '？' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            push_sentence(std::mem::take(&mut sentence));
        }
    }
    if !sentence.is_empty() {
        push_sentence(sentence);
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
        .into_iter()
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_a_single_trimmed_chunk() {
        assert_eq!(
            split_text("  안녕하세요. 반갑습니다.\n", 4000),
            ["안녕하세요. 반갑습니다."]
        );
        assert!(split_text("", 4000).is_empty());
        assert!(split_text(" \n\t \n", 4000).is_empty());
    }

    #[test]
    fn sentences_are_packed_up_to_the_limit_without_being_cut() {
        let text = "Alpha one. Bravo two! Charlie three? Delta four.";
        // Sentences keep their leading space (10, 11, 15 and 12 chars): a limit of 28 packs
        // two sentences per chunk.
        let chunks = split_text(text, 28);
        assert_eq!(
            chunks,
            ["Alpha one. Bravo two!", "Charlie three? Delta four."]
        );
        assert!(chunks.iter().all(|c| c.chars().count() <= 28));
    }

    #[test]
    fn chunks_in_order_cover_the_whole_text() {
        let sentences: Vec<String> = (0..50).map(|i| format!("Sentence number {i}.")).collect();
        let text = sentences.join(" ");
        let chunks = split_text(&text, 120);
        assert!(chunks.len() > 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 120));
        assert_eq!(chunks.join(" "), text);
    }

    #[test]
    fn cjk_terminators_and_newlines_split_without_spaces() {
        let text = "첫 문장입니다。둘째 문장입니다！셋째 문장입니까？넷째\n다섯째";
        let chunks = split_text(text, 14);
        assert_eq!(chunks.concat().replace('\n', ""), text.replace('\n', ""));
        assert!(chunks.iter().all(|c| c.chars().count() <= 14), "{chunks:?}");
        assert!(chunks[0].ends_with('。'));
    }

    #[test]
    fn periods_inside_numbers_and_abbreviations_do_not_end_sentences() {
        // `3.14` has no whitespace after the period, so it is never a boundary.
        let text = "Pi is 3.14 roughly";
        assert_eq!(split_text(text, 4000), [text]);
        let chunks = split_text("version 3.14 is out now", 16);
        // Without boundaries the whole text is one oversized "sentence" and is hard-split.
        assert!(chunks.iter().all(|c| c.chars().count() <= 16));
        assert_eq!(chunks.concat().replace(' ', ""), "version3.14isoutnow");
    }

    #[test]
    fn an_oversized_sentence_is_hard_split() {
        let long = "가".repeat(9500); // one "sentence" without terminators
        let chunks = split_text(&long, 4000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chars().count(), 4000);
        assert_eq!(chunks[1].chars().count(), 4000);
        assert_eq!(chunks[2].chars().count(), 1500);
        assert_eq!(chunks.concat(), long);
    }

    #[test]
    fn short_sentences_after_an_oversized_one_join_its_tail() {
        let text = format!("{} Tail sentence.", "x".repeat(4500));
        let chunks = split_text(&text, 4000);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].ends_with("Tail sentence."));
        assert!(chunks.iter().all(|c| c.chars().count() <= 4000));
    }

    #[test]
    fn every_chunk_is_within_the_limit_for_mixed_text() {
        let text = "문장 하나입니다. ".repeat(900) + &"Another one! ".repeat(400);
        let chunks = split_text(&text, MAX_INPUT_CHARS);
        assert!(chunks.len() >= 3);
        assert!(
            chunks
                .iter()
                .all(|c| !c.is_empty() && c.chars().count() <= MAX_INPUT_CHARS)
        );
    }

    #[test]
    fn debug_hides_the_key_and_messages_match_the_contract() {
        let client = TtsClient::new(
            reqwest::Client::new(),
            "http://x",
            Some("sk-secret".into()),
            "tts-1",
            "alloy",
        );
        assert!(!format!("{client:?}").contains("sk-secret"));
        assert_eq!(
            TtsError::MissingApiKey.to_string(),
            "TTS_API_KEY 가 설정되지 않았습니다"
        );
    }
}
