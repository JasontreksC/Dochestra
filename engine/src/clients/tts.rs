//! Text-to-speech client for OpenRouter's OpenAI-compatible `/audio/speech` API (CONTEXT §4.3).
//!
//! The configured TTS model returns raw 16-bit PCM only; the engine joins the chunks and
//! encodes the result as one MP3 (LAME, statically linked — no external binary needed).

use std::time::Duration;

use std::mem::MaybeUninit;

use mp3lame_encoder::{
    Bitrate, Builder, FlushNoGap, InterleavedPcm, MonoPcm, Quality, max_required_buffer_size,
};
use serde_json::json;

use super::common::{
    DEFAULT_BACKOFF_BASE, Failure, error_message, parse_retry_after, retry, scrub_secret,
};

/// Maximum characters per synthesis request.
pub const MAX_INPUT_CHARS: usize = 4000;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Used when the response does not state its format (observed: 24 kHz mono).
const DEFAULT_RATE: u32 = 24_000;
const DEFAULT_CHANNELS: u16 = 1;
const MONO_BITRATE: Bitrate = Bitrate::Kbps64;
const STEREO_BITRATE: Bitrate = Bitrate::Kbps128;
/// LAME needs at least 7200 bytes for the final flush.
const FLUSH_BUFFER_BYTES: usize = 8192;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TtsError {
    #[error("OPENROUTER_API_KEY 가 설정되지 않았습니다")]
    MissingApiKey,
    #[error("TTS API 오류 (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    #[error("TTS API 에 연결할 수 없습니다: {0}")]
    Transport(String),
    #[error("TTS 응답을 해석할 수 없습니다: {0}")]
    Decode(String),
    #[error("MP3 인코딩에 실패했습니다: {0}")]
    Encode(String),
}

/// One synthesized chunk: 16-bit little-endian PCM with its format.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PcmChunk {
    rate: u32,
    channels: u16,
    samples: Vec<i16>,
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
    /// Text over 4000 characters is split at sentence boundaries and synthesized chunk by chunk
    /// in order; the PCM of all chunks (which must share one format) is joined and encoded as a
    /// single MP3. Blank text yields no audio (and needs no key).
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let chunks = split_text(text, MAX_INPUT_CHARS);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let Some(api_key) = self.api_key.as_deref() else {
            return Err(TtsError::MissingApiKey);
        };

        let mut joined: Option<PcmChunk> = None;
        for chunk in &chunks {
            let pcm = self.synthesize_chunk(api_key, chunk).await?;
            match &mut joined {
                None => joined = Some(pcm),
                Some(all) if (all.rate, all.channels) == (pcm.rate, pcm.channels) => {
                    all.samples.extend(pcm.samples);
                }
                Some(all) => {
                    return Err(TtsError::Decode(format!(
                        "청크마다 오디오 형식이 다릅니다 ({} Hz/{}ch 와 {} Hz/{}ch)",
                        all.rate, all.channels, pcm.rate, pcm.channels
                    )));
                }
            }
        }
        let Some(pcm) = joined else {
            return Ok(Vec::new());
        };
        tokio::task::spawn_blocking(move || encode_mp3(&pcm.samples, pcm.rate, pcm.channels))
            .await
            .map_err(|e| TtsError::Encode(e.to_string()))?
    }

    async fn synthesize_chunk(&self, api_key: &str, input: &str) -> Result<PcmChunk, TtsError> {
        let url = format!("{}/audio/speech", self.base_url);
        let body = json!({
            "model": self.model,
            "input": input,
            "voice": self.voice,
            "response_format": "pcm",
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
    ) -> Result<PcmChunk, Failure<TtsError>> {
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .bearer_auth(api_key)
            .header("X-Title", "Dochestra")
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
            .to_owned();
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
        parse_pcm(&content_type, &bytes).map_err(Failure::fatal)
    }
}

/// Parses `audio/pcm;rate=24000;channels=1`-style content types: `(rate, channels)` as far as
/// they are present. `None` when the type is not `audio/*`.
pub fn parse_audio_format(content_type: &str) -> Option<(Option<u32>, Option<u16>)> {
    let mut parts = content_type.split(';');
    let media = parts.next()?.trim().to_ascii_lowercase();
    if !media.starts_with("audio/") {
        return None;
    }
    let (mut rate, mut channels) = (None, None);
    for param in parts {
        let Some((key, value)) = param.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
            "rate" => rate = value.parse().ok(),
            "channels" => channels = value.parse().ok(),
            _ => {}
        }
    }
    Some((rate, channels))
}

/// Interprets a successful response body as 16-bit little-endian PCM.
fn parse_pcm(content_type: &str, bytes: &[u8]) -> Result<PcmChunk, TtsError> {
    let Some((rate, channels)) = parse_audio_format(content_type) else {
        return Err(TtsError::Decode(format!(
            "오디오가 아닌 응답입니다 (Content-Type: {})",
            if content_type.is_empty() {
                "없음"
            } else {
                content_type
            }
        )));
    };
    if rate.is_none() || channels.is_none() {
        tracing::warn!(
            content_type,
            "TTS response lacks rate/channels; assuming 24 kHz mono"
        );
    }
    let (rate, channels) = (
        rate.unwrap_or(DEFAULT_RATE),
        channels.unwrap_or(DEFAULT_CHANNELS),
    );
    if bytes.is_empty() {
        return Err(TtsError::Decode("응답 본문이 비어 있습니다".to_owned()));
    }
    let frame = 2 * usize::from(channels.max(1));
    if !bytes.len().is_multiple_of(frame) {
        return Err(TtsError::Decode(format!(
            "PCM 길이({}바이트)가 {frame}바이트 단위가 아닙니다",
            bytes.len()
        )));
    }
    let samples = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    Ok(PcmChunk {
        rate,
        channels,
        samples,
    })
}

/// Appends the first `written` bytes of an encoder output buffer.
fn append_written(out: &mut Vec<u8>, buffer: &[MaybeUninit<u8>], written: usize) {
    // SAFETY: every byte of `buffer` was initialized to 0 when it was created, so reading any of
    // them is sound; LAME overwrote the first `written` bytes with the encoded data.
    let bytes = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), written.min(buffer.len()))
    };
    out.extend_from_slice(bytes);
}

/// Encodes 16-bit PCM (interleaved when stereo) as MP3: 64 kbps mono, 128 kbps stereo.
pub fn encode_mp3(samples: &[i16], rate: u32, channels: u16) -> Result<Vec<u8>, TtsError> {
    let fail = |what: &str, err: &dyn std::fmt::Debug| TtsError::Encode(format!("{what}: {err:?}"));
    if !(1..=2).contains(&channels) {
        return Err(TtsError::Encode(format!(
            "지원하지 않는 채널 수입니다: {channels}"
        )));
    }
    let mut builder = Builder::new()
        .ok_or_else(|| TtsError::Encode("LAME 을 초기화할 수 없습니다".to_owned()))?;
    builder
        .set_num_channels(channels as u8)
        .map_err(|e| fail("채널 설정", &e))?;
    builder
        .set_sample_rate(rate)
        .map_err(|e| fail("샘플레이트 설정", &e))?;
    builder
        .set_brate(if channels == 1 {
            MONO_BITRATE
        } else {
            STEREO_BITRATE
        })
        .map_err(|e| fail("비트레이트 설정", &e))?;
    builder
        .set_quality(Quality::Good)
        .map_err(|e| fail("품질 설정", &e))?;
    let mut encoder = builder.build().map_err(|e| fail("인코더 생성", &e))?;

    // `encode_to_vec` of mp3lame-encoder 0.3.0 crashes (SIGSEGV), so encode into explicit buffers.
    // Blocks of ~1 second keep the buffers small.
    let samples_per_frame = usize::from(channels);
    let block_frames = rate.max(8_000) as usize;
    let mut out: Vec<u8> = Vec::new();
    for block in samples.chunks(block_frames * samples_per_frame) {
        let frames = block.len() / samples_per_frame;
        let mut buffer = vec![MaybeUninit::<u8>::new(0); max_required_buffer_size(frames)];
        let written = if channels == 1 {
            encoder.encode(MonoPcm(block), &mut buffer)
        } else {
            encoder.encode(InterleavedPcm(block), &mut buffer)
        }
        .map_err(|e| fail("인코딩", &e))?;
        append_written(&mut out, &buffer, written);
    }
    let mut tail = vec![MaybeUninit::<u8>::new(0); FLUSH_BUFFER_BYTES];
    let written = encoder
        .flush::<FlushNoGap>(&mut tail)
        .map_err(|e| fail("마무리", &e))?;
    append_written(&mut out, &tail, written);
    Ok(out)
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
            "OPENROUTER_API_KEY 가 설정되지 않았습니다"
        );
    }

    #[test]
    fn content_types_are_parsed() {
        assert_eq!(
            parse_audio_format("audio/pcm;rate=24000;channels=1"),
            Some((Some(24000), Some(1)))
        );
        assert_eq!(
            parse_audio_format("Audio/PCM; Rate=\"16000\" ; channels=2"),
            Some((Some(16000), Some(2)))
        );
        assert_eq!(parse_audio_format("audio/pcm"), Some((None, None)));
        assert_eq!(
            parse_audio_format("audio/L16;rate=44100"),
            Some((Some(44100), None))
        );
        assert_eq!(
            parse_audio_format("audio/pcm;rate=abc;channels=1"),
            Some((None, Some(1)))
        );
        assert_eq!(parse_audio_format("application/json"), None);
        assert_eq!(parse_audio_format(""), None);
    }

    #[test]
    fn pcm_bodies_are_decoded_as_little_endian_samples() {
        let pcm = parse_pcm(
            "audio/pcm;rate=24000;channels=1",
            &[0x01, 0x00, 0xFF, 0xFF, 0x00, 0x80],
        )
        .unwrap();
        assert_eq!(
            pcm,
            PcmChunk {
                rate: 24000,
                channels: 1,
                samples: vec![1, -1, i16::MIN]
            }
        );
        // Missing format parameters fall back to 24 kHz mono.
        let pcm = parse_pcm("audio/pcm", &[0, 0, 1, 0]).unwrap();
        assert_eq!((pcm.rate, pcm.channels, pcm.samples.len()), (24000, 1, 2));
        // Stereo frames are 4 bytes.
        assert!(parse_pcm("audio/pcm;rate=24000;channels=2", &[0; 8]).is_ok());
        assert!(matches!(
            parse_pcm("audio/pcm;rate=24000;channels=2", &[0; 6]),
            Err(TtsError::Decode(_))
        ));
        assert!(matches!(
            parse_pcm("audio/pcm;rate=24000;channels=1", &[0; 3]),
            Err(TtsError::Decode(_))
        ));
        assert!(matches!(
            parse_pcm("audio/pcm;rate=24000;channels=1", &[]),
            Err(TtsError::Decode(_))
        ));
        assert!(matches!(
            parse_pcm("application/json", &[0, 0]),
            Err(TtsError::Decode(_))
        ));
    }

    #[test]
    fn unsupported_channel_counts_fail_to_encode() {
        assert!(matches!(
            encode_mp3(&[0; 6], 24000, 3),
            Err(TtsError::Encode(_))
        ));
        assert!(matches!(
            encode_mp3(&[0; 6], 24000, 0),
            Err(TtsError::Encode(_))
        ));
    }

    /// Decodes an MP3 with symphonia: `(sample rate, first channel as f32)`.
    fn decode_mp3(bytes: &[u8]) -> (u32, Vec<f32>) {
        use symphonia::core::audio::sample::Sample;
        use symphonia::core::codecs::audio::AudioDecoderOptions;
        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;

        let source = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(bytes.to_vec())),
            Default::default(),
        );
        let mut hint = Hint::new();
        hint.with_extension("mp3");
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                source,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .expect("a valid MPEG audio stream");
        let track = format.default_track(TrackType::Audio).expect("audio track");
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make_audio_decoder(
                track.codec_params.as_ref().unwrap().audio().unwrap(),
                &AudioDecoderOptions::default(),
            )
            .expect("MP3 decoder");

        let (mut rate, mut samples) = (0u32, Vec::new());
        while let Some(packet) = format.next_packet().expect("readable packet") {
            if packet.track_id != track_id {
                continue;
            }
            let audio = decoder.decode(&packet).expect("decodable packet");
            rate = audio.spec().rate();
            let channels = audio.spec().channels().count();
            let mut interleaved = vec![f32::MID; audio.samples_interleaved()];
            audio.copy_to_slice_interleaved(&mut interleaved);
            samples.extend(interleaved.chunks(channels).map(|frame| frame[0]));
        }
        (rate, samples)
    }

    /// A sine wave of `seconds` at `freq` Hz, 16-bit.
    fn sine(rate: u32, seconds: f32, freq: f32) -> Vec<i16> {
        (0..(rate as f32 * seconds) as usize)
            .map(|n| {
                ((n as f32 / rate as f32 * freq * std::f32::consts::TAU).sin()
                    * 0.5
                    * f32::from(i16::MAX)) as i16
            })
            .collect()
    }

    #[test]
    fn mono_pcm_becomes_a_valid_mp3_that_decodes_back_to_the_tone() {
        let (rate, seconds, freq) = (24_000u32, 2.0f32, 440.0f32);
        let mp3 = encode_mp3(&sine(rate, seconds, freq), rate, 1).unwrap();

        // Starts with an MPEG audio frame sync, and the size matches 64 kbps for 2 seconds.
        assert_eq!(mp3[0], 0xFF);
        assert_eq!(mp3[1] & 0xE0, 0xE0, "frame sync");
        let expected = (64_000.0 / 8.0 * seconds) as usize;
        assert!(
            (expected * 8 / 10..expected * 13 / 10).contains(&mp3.len()),
            "{} bytes for ~{expected}",
            mp3.len()
        );

        // It decodes: right sample rate, about the right duration, audible, and the right pitch.
        let (decoded_rate, decoded) = decode_mp3(&mp3);
        assert_eq!(decoded_rate, rate);
        let expected_samples = (rate as f32 * seconds) as usize;
        assert!(
            decoded.len().abs_diff(expected_samples) <= 3 * 576, // encoder delay/padding: a few frames
            "{} samples for {expected_samples}",
            decoded.len()
        );
        let steady = &decoded[2000..decoded.len() - 2000];
        let rms = (steady.iter().map(|x| x * x).sum::<f32>() / steady.len() as f32).sqrt();
        assert!(
            (0.25..0.45).contains(&rms),
            "rms {rms} (a 0.5 amplitude sine has ~0.354)"
        );
        let crossings = steady
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count() as f32;
        let measured_hz = crossings / (steady.len() as f32 / rate as f32);
        assert!((measured_hz - freq).abs() < freq * 0.02, "{measured_hz} Hz");
    }

    #[test]
    fn mp3_length_grows_with_the_audio_length() {
        let short = encode_mp3(&sine(24_000, 1.0, 300.0), 24_000, 1)
            .unwrap()
            .len();
        let long = encode_mp3(&sine(24_000, 4.0, 300.0), 24_000, 1)
            .unwrap()
            .len();
        let ratio = long as f32 / short as f32;
        assert!((3.2..4.8).contains(&ratio), "ratio {ratio}");
    }

    #[test]
    fn stereo_pcm_is_encoded_at_a_higher_bitrate() {
        let mono = sine(24_000, 1.0, 440.0);
        let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s]).collect();
        let mp3 = encode_mp3(&stereo, 24_000, 2).unwrap();
        let (decoded_rate, _) = decode_mp3(&mp3);
        assert_eq!(decoded_rate, 24_000);
        let expected = 128_000 / 8; // one second
        assert!(
            (expected * 8 / 10..expected * 13 / 10).contains(&mp3.len()),
            "{}",
            mp3.len()
        );
    }

    #[test]
    fn unusual_sample_rates_are_encoded_too() {
        // LAME resamples input rates MP3 does not support (here 22.05 kHz is native, 20 kHz is not).
        for rate in [22_050u32, 16_000, 44_100, 20_000] {
            let mp3 = encode_mp3(&sine(rate, 0.5, 440.0), rate, 1)
                .unwrap_or_else(|e| panic!("{rate}: {e}"));
            assert!(mp3.len() > 500 && mp3[0] == 0xFF, "{rate}");
        }
    }
}
