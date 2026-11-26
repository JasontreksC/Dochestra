use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use dochestra_engine::build_app;
use dochestra_engine::clients::llm::LlmClient;
use dochestra_engine::clients::tts::{MAX_INPUT_CHARS, TtsClient, TtsError};
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs,
};
use dochestra_engine::nodes::{EngineRunner, voice};
use dochestra_engine::state::AppState;
use dochestra_engine::types::{DataType, NodeKind, ScalarType};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const KEY: &str = "sk-or-secret-1";
const RATE: u32 = 24_000;
/// Every mock TTS answer is a tone of this length.
const CHUNK_SECONDS: f32 = 0.5;

// ------------------------------------------------------------------ mock OpenRouter

#[derive(Clone)]
struct Reply {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    headers: Vec<(&'static str, &'static str)>,
}

struct Recorded {
    headers: HashMap<String, String>,
    body: Value,
}

type Responder = Arc<dyn Fn(&Value) -> Reply + Send + Sync>;

#[derive(Clone)]
struct Shared {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    default: Responder,
}

struct Mock {
    url: String,
    shared: Shared,
}

impl Mock {
    fn count(&self) -> usize {
        self.shared.requests.lock().unwrap().len()
    }
    fn body(&self, i: usize) -> Value {
        self.shared.requests.lock().unwrap()[i].body.clone()
    }
    fn header(&self, i: usize, name: &str) -> Option<String> {
        self.shared.requests.lock().unwrap()[i]
            .headers
            .get(name)
            .cloned()
    }
    /// The `input` text of every TTS request, in order.
    fn tts_inputs(&self) -> Vec<String> {
        (0..self.count())
            .map(|i| self.body(i)["input"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// One server answering several endpoints under one base URL (as OpenRouter does).
async fn spawn_routes(routes: Vec<(&'static str, Vec<Reply>, Responder)>) -> Vec<Mock> {
    let mut app = Router::new();
    let mut shares = Vec::new();
    for (path, scripted, default) in routes {
        let shared = Shared {
            replies: Arc::new(Mutex::new(scripted.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
            default,
        };
        app = app.route(path, post(handler).with_state(shared.clone()));
        shares.push(shared);
    }
    let app = app.layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    shares
        .into_iter()
        .map(|shared| Mock {
            url: url.clone(),
            shared,
        })
        .collect()
}

async fn handler(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let recorded_headers = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                v.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let reply = shared
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| (shared.default)(&body));
    shared.requests.lock().unwrap().push(Recorded {
        headers: recorded_headers,
        body,
    });
    let mut resp = (StatusCode::from_u16(reply.status).unwrap(), reply.body).into_response();
    resp.headers_mut()
        .insert("content-type", reply.content_type.parse().unwrap());
    for (name, value) in reply.headers {
        resp.headers_mut().insert(name, value.parse().unwrap());
    }
    resp
}

fn json_error(status: u16, message: &str) -> Reply {
    Reply {
        status,
        content_type: "application/json".to_owned(),
        body: json!({"error": {"message": message, "code": status}})
            .to_string()
            .into_bytes(),
        headers: vec![],
    }
}

/// Little-endian 16-bit PCM of a sine tone.
fn tone_pcm(rate: u32, seconds: f32, freq: f32) -> Vec<u8> {
    (0..(rate as f32 * seconds) as usize)
        .flat_map(|n| {
            let v = (n as f32 / rate as f32 * freq * std::f32::consts::TAU).sin()
                * 0.5
                * f32::from(i16::MAX);
            (v as i16).to_le_bytes()
        })
        .collect()
}

fn pcm_reply(content_type: &str, pcm: Vec<u8>) -> Reply {
    Reply {
        status: 200,
        content_type: content_type.to_owned(),
        body: pcm,
        headers: vec![],
    }
}

/// Chunk number `k` (0-based, in request order) is a tone of `300 + 150 k` Hz, so the order in
/// which chunks were joined can be heard in the final MP3.
fn tone_hz(k: usize) -> f32 {
    300.0 + 150.0 * k as f32
}

fn tts_default() -> Responder {
    let counter = Arc::new(AtomicUsize::new(0));
    Arc::new(move |_body| {
        let k = counter.fetch_add(1, Ordering::SeqCst);
        pcm_reply(
            "audio/pcm;rate=24000;channels=1",
            tone_pcm(RATE, CHUNK_SECONDS, tone_hz(k)),
        )
    })
}

fn llm_reply(text: &str) -> Reply {
    Reply {
        status: 200,
        content_type: "application/json".to_owned(),
        body: json!({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]})
            .to_string()
            .into_bytes(),
        headers: vec![],
    }
}

fn llm_default() -> Responder {
    Arc::new(|_| llm_reply("안녕하세요. 이 문서를 설명해 드리겠습니다."))
}

/// `(llm, tts)` behind one base URL.
async fn spawn_providers(llm_replies: Vec<Reply>, tts_replies: Vec<Reply>) -> (Mock, Mock) {
    let mut mocks = spawn_routes(vec![
        ("/chat/completions", llm_replies, llm_default()),
        ("/audio/speech", tts_replies, tts_default()),
    ])
    .await;
    let tts = mocks.remove(1);
    (mocks.remove(0), tts)
}

async fn spawn_tts(scripted: Vec<Reply>) -> Mock {
    spawn_routes(vec![("/audio/speech", scripted, tts_default())])
        .await
        .remove(0)
}

// ------------------------------------------------------------------ MP3 inspection

struct Decoded {
    rate: u32,
    samples: Vec<f32>,
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

fn decode(mp3: &[u8]) -> Decoded {
    assert!(
        mp3.len() > 100 && mp3[0] == 0xFF && mp3[1] & 0xE0 == 0xE0,
        "not an MPEG audio stream"
    );
    let (rate, samples) = decode_mp3(mp3);
    Decoded { rate, samples }
}

/// Which of the `candidates` (Hz) dominates `samples[from..to]`, by Goertzel energy.
fn dominant(decoded: &Decoded, from: usize, to: usize, candidates: &[f32]) -> f32 {
    let window = &decoded.samples[from..to];
    let energy = |freq: f32| {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, x) in window.iter().enumerate() {
            let angle = std::f32::consts::TAU * freq * n as f32 / decoded.rate as f32;
            re += x * angle.cos();
            im -= x * angle.sin();
        }
        re * re + im * im
    };
    candidates
        .iter()
        .copied()
        .max_by(|a, b| energy(*a).total_cmp(&energy(*b)))
        .expect("candidates")
}

// ------------------------------------------------------------------ TTS client (direct)

fn tts_client(url: &str, key: Option<&str>) -> TtsClient {
    TtsClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "google/gemini-3.8-flash-lite-tts",
        "Kore",
    )
    .with_backoff_base(Duration::from_millis(5))
}

#[tokio::test]
async fn tts_requests_pcm_and_returns_a_valid_mp3() {
    let mock = spawn_tts(vec![]).await;
    let mp3 = tts_client(&mock.url, Some(KEY))
        .synthesize("Hello there.")
        .await
        .unwrap();

    assert_eq!(mock.count(), 1);
    assert_eq!(
        mock.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );
    assert!(
        mock.header(0, "content-type")
            .unwrap()
            .starts_with("application/json")
    );
    assert_eq!(
        mock.body(0),
        json!({"model": "google/gemini-3.8-flash-lite-tts", "input": "Hello there.", "voice": "Kore", "response_format": "pcm"})
    );

    let audio = decode(&mp3);
    assert_eq!(audio.rate, RATE);
    let expected = (RATE as f32 * CHUNK_SECONDS) as usize;
    assert!(
        audio.samples.len().abs_diff(expected) <= 3 * 576,
        "{} samples for {expected}",
        audio.samples.len()
    );
    assert_eq!(
        dominant(
            &audio,
            2000,
            expected - 2000,
            &[tone_hz(0), tone_hz(1), tone_hz(2)]
        ),
        tone_hz(0)
    );
}

#[tokio::test]
async fn long_text_is_synthesized_in_order_and_joined_into_one_mp3() {
    let mock = spawn_tts(vec![]).await;
    // 1200 sentences of 15 chars = 18,000 chars -> at least 5 requests of <= 4000 chars.
    let text: String = (0..1200).map(|i| format!("S{i:04} is here. ")).collect();
    let mp3 = tts_client(&mock.url, Some(KEY))
        .synthesize(&text)
        .await
        .unwrap();

    let inputs = mock.tts_inputs();
    let chunks = inputs.len();
    assert!(chunks >= 5, "{chunks} requests");
    assert!(
        inputs
            .iter()
            .all(|i| i.chars().count() <= MAX_INPUT_CHARS && !i.is_empty())
    );
    // Requests were sequential and in text order, covering every sentence exactly once.
    let first_ids: Vec<usize> = inputs.iter().map(|i| i[1..5].parse().unwrap()).collect();
    assert!(first_ids.windows(2).all(|w| w[0] < w[1]), "{first_ids:?}");
    assert_eq!(inputs.join(" "), text.trim());

    // One MP3 with the chunks back to back: total length is the sum, and the pitch steps up in
    // request order (chunk k is a tone of 300 + 150 k Hz).
    let audio = decode(&mp3);
    let per_chunk = (RATE as f32 * CHUNK_SECONDS) as usize;
    assert!(
        audio.samples.len().abs_diff(per_chunk * chunks) <= 3 * 576,
        "{} samples",
        audio.samples.len()
    );
    let candidates: Vec<f32> = (0..chunks).map(tone_hz).collect();
    for k in 0..chunks {
        let (from, to) = (
            k * per_chunk + per_chunk / 5,
            (k + 1) * per_chunk - per_chunk / 5,
        );
        assert_eq!(
            dominant(&audio, from, to, &candidates),
            tone_hz(k),
            "chunk {k}"
        );
    }
}

#[tokio::test]
async fn tts_errors_retries_and_key_handling() {
    // 401: message propagated, not retried, key scrubbed even when echoed.
    let mock = spawn_tts(vec![json_error(401, &format!("Incorrect API key: {KEY}"))]).await;
    let err = tts_client(&mock.url, Some(KEY))
        .synthesize("hi.")
        .await
        .unwrap_err();
    assert!(matches!(err, TtsError::Api { status: 401, .. }));
    let text = err.to_string();
    assert!(
        text.starts_with("TTS API 오류 (HTTP 401): Incorrect API key"),
        "{text}"
    );
    assert!(!text.contains(KEY) && !format!("{err:?}").contains(KEY));
    assert_eq!(mock.count(), 1);

    // The documented model/voice mismatch (HTTP 400) is reported as such, without retry.
    let mock = spawn_tts(vec![json_error(400, "Voice 'alloy' is not supported")]).await;
    let err = tts_client(&mock.url, Some(KEY))
        .synthesize("hi.")
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "TTS API 오류 (HTTP 400): Voice 'alloy' is not supported"
    );
    assert_eq!(mock.count(), 1);

    // 429 (retry-after: 0) and 503 are retried (shared retry policy).
    let mut limited = json_error(429, "slow");
    limited.headers.push(("retry-after", "0"));
    let mock = spawn_tts(vec![limited, json_error(503, "busy")]).await;
    assert!(
        !tts_client(&mock.url, Some(KEY))
            .synthesize("retry me.")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(mock.count(), 3);

    // Missing key: no HTTP call; blank text needs neither key nor call.
    let mock = spawn_tts(vec![]).await;
    let err = tts_client(&mock.url, None)
        .synthesize("hi.")
        .await
        .unwrap_err();
    assert_eq!(err, TtsError::MissingApiKey);
    assert_eq!(err.to_string(), "OPENROUTER_API_KEY 가 설정되지 않았습니다");
    assert!(
        tts_client(&mock.url, None)
            .synthesize("  \n ")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn the_audio_format_comes_from_the_response_header() {
    // 16 kHz instead of 24 kHz: the MP3 must be encoded at the stated rate.
    let reply = pcm_reply(
        "audio/pcm;rate=16000;channels=1",
        tone_pcm(16_000, 0.5, 500.0),
    );
    let mock = spawn_tts(vec![reply]).await;
    let audio = decode(
        &tts_client(&mock.url, Some(KEY))
            .synthesize("hi.")
            .await
            .unwrap(),
    );
    assert_eq!(audio.rate, 16_000);
    assert_eq!(dominant(&audio, 1500, 6500, &[300.0, 500.0, 700.0]), 500.0);

    // Without rate/channels the documented default (24 kHz mono) is assumed.
    let reply = pcm_reply("audio/pcm", tone_pcm(RATE, 0.5, 400.0));
    let mock = spawn_tts(vec![reply]).await;
    assert_eq!(
        decode(
            &tts_client(&mock.url, Some(KEY))
                .synthesize("hi.")
                .await
                .unwrap()
        )
        .rate,
        RATE
    );
}

#[tokio::test]
async fn chunks_with_different_formats_are_rejected() {
    let first = pcm_reply(
        "audio/pcm;rate=24000;channels=1",
        tone_pcm(RATE, 0.25, 440.0),
    );
    let second = pcm_reply(
        "audio/pcm;rate=16000;channels=1",
        tone_pcm(16_000, 0.25, 440.0),
    );
    let mock = spawn_tts(vec![first, second]).await;
    let text: String = (0..800).map(|i| format!("S{i:04} is here. ")).collect(); // two+ chunks
    let err = tts_client(&mock.url, Some(KEY))
        .synthesize(&text)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, TtsError::Decode(m) if m.contains("오디오 형식이 다릅니다")),
        "{err:?}"
    );
}

#[tokio::test]
async fn unusable_audio_responses_are_decode_errors() {
    let cases = [
        pcm_reply("application/json", b"{}".to_vec()), // not audio
        pcm_reply("audio/pcm;rate=24000;channels=1", vec![]), // empty
        pcm_reply("audio/pcm;rate=24000;channels=1", vec![0, 0, 0]), // odd byte count
        pcm_reply("audio/pcm;rate=24000;channels=2", vec![0, 0, 0, 0, 0, 0]), // not whole frames
    ];
    for reply in cases {
        let mock = spawn_tts(vec![reply]).await;
        let err = tts_client(&mock.url, Some(KEY))
            .synthesize("hi.")
            .await
            .unwrap_err();
        assert!(matches!(err, TtsError::Decode(_)), "{err:?}");
        assert!(
            err.to_string()
                .starts_with("TTS 응답을 해석할 수 없습니다: ")
        );
        assert_eq!(mock.count(), 1, "decode errors are not retried");
    }
}

// ------------------------------------------------------------------ voice node (direct)

struct Env {
    dir: PathBuf,
    ctx: NodeContext,
}

impl Env {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("dochestra-voice-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = NodeContext {
            run_id: Uuid::new_v4(),
            node_id: "v".to_owned(),
            kind: NodeKind::Voice,
            params: json!({}),
            out_dir: dir.clone(),
        };
        Self { dir, ctx }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn inputs(&self, image_count: usize) -> NodeInputs {
        let images: Vec<PathBuf> = (0..image_count)
            .map(|i| self.file(&format!("img{i:02}.jpg"), &[0xff, 0xd8, i as u8]))
            .collect();
        NodeInputs::from([
            (
                "md".to_owned(),
                Artifact {
                    data_type: DataType::single(ScalarType::Md),
                    paths: vec![self.file("doc.md", "# 제목\n\n본문 내용입니다.".as_bytes())],
                },
            ),
            (
                "images".to_owned(),
                Artifact {
                    data_type: DataType::list_of(ScalarType::Jpg),
                    paths: images,
                },
            ),
        ])
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn llm_client(url: &str, key: Option<&str>) -> LlmClient {
    LlmClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "meta/muse-ember",
        "medium",
    )
    .with_backoff_base(Duration::from_millis(5))
}

fn failed(result: Result<NodeOutputs, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(message)) => message,
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// Content parts of the user message of LLM request 0.
fn llm_content(mock: &Mock) -> Vec<Value> {
    mock.body(0)["messages"][1]["content"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn voice_writes_a_script_then_the_mp3() {
    let env = Env::new();
    let script = "안녕하세요. 이 문서를 설명해 드리겠습니다.";
    let (llm, tts) = spawn_providers(vec![llm_reply(&format!("\n{script}\n"))], vec![]).await;
    let out = voice::run(
        &llm_client(&llm.url, Some(KEY)),
        &tts_client(&tts.url, Some(KEY)),
        &env.ctx,
        &env.inputs(2),
    )
    .await
    .unwrap();

    // Output: one MP3 artifact; its audio is what the TTS model returned.
    assert_eq!(out.len(), 1);
    assert_eq!(out["mp3"].data_type, DataType::single(ScalarType::Mp3));
    assert_eq!(out["mp3"].paths, vec![env.dir.join("mp3.mp3")]);
    let audio = decode(&std::fs::read(&out["mp3"].paths[0]).unwrap());
    assert_eq!(audio.rate, RATE);
    assert!(audio.samples.len() > 10_000);

    // The (trimmed) script is kept in _work and is exactly what was synthesized.
    assert_eq!(
        std::fs::read_to_string(env.dir.join("_work/script.txt")).unwrap(),
        script
    );
    assert_eq!(tts.tts_inputs(), [script]);

    // The narration request: both images (data URLs) before the text, narrator system prompt.
    let content = llm_content(&llm);
    let types: Vec<&str> = content
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["image_url", "image_url", "text"]);
    assert!(
        content[0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    let text = content[2]["text"].as_str().unwrap();
    assert!(
        text.contains("<document>\n# 제목\n\n본문 내용입니다.\n</document>"),
        "{text}"
    );
    assert!(text.contains("첨부된 이미지 2개"));
    let system = llm.body(0)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        system.contains("내레이터")
            && system.contains("마크다운")
            && system.contains("1500~2500자")
    );
    assert_eq!(
        llm.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );
}

#[tokio::test]
async fn voice_without_images_and_with_many_images() {
    let env = Env::new();
    let (llm, tts) = spawn_providers(vec![], vec![]).await;
    voice::run(
        &llm_client(&llm.url, Some(KEY)),
        &tts_client(&tts.url, Some(KEY)),
        &env.ctx,
        &env.inputs(0),
    )
    .await
    .unwrap();
    let content = llm_content(&llm);
    assert_eq!(content.len(), 1);
    assert!(
        !content[0]["text"]
            .as_str()
            .unwrap()
            .contains("첨부된 이미지")
    );

    // Same image rules as summary: at most 20; the rest is left out (and not read aloud).
    let env = Env::new();
    let (llm, tts) = spawn_providers(vec![], vec![]).await;
    voice::run(
        &llm_client(&llm.url, Some(KEY)),
        &tts_client(&tts.url, Some(KEY)),
        &env.ctx,
        &env.inputs(21),
    )
    .await
    .unwrap();
    let images = llm_content(&llm)
        .iter()
        .filter(|b| b["type"] == "image_url")
        .count();
    assert_eq!(images, 20);
    assert!(
        !std::fs::read_to_string(env.dir.join("_work/script.txt"))
            .unwrap()
            .contains("제외")
    );
    assert_eq!(tts.count(), 1);
}

#[tokio::test]
async fn a_long_script_is_synthesized_in_several_requests_and_joined() {
    let env = Env::new();
    let script: String = (0..700).map(|i| format!("S{i:03} 문장이에요. ")).collect();
    let (llm, tts) = spawn_providers(vec![llm_reply(&script)], vec![]).await;
    let out = voice::run(
        &llm_client(&llm.url, Some(KEY)),
        &tts_client(&tts.url, Some(KEY)),
        &env.ctx,
        &env.inputs(0),
    )
    .await
    .unwrap();
    let chunks = tts.count();
    assert!(chunks >= 2, "{chunks} TTS requests");
    let audio = decode(&std::fs::read(&out["mp3"].paths[0]).unwrap());
    let per_chunk = (RATE as f32 * CHUNK_SECONDS) as usize;
    assert!(
        audio.samples.len().abs_diff(per_chunk * chunks) <= 3 * 576,
        "{} samples",
        audio.samples.len()
    );
}

#[tokio::test]
async fn an_empty_script_fails_without_calling_tts() {
    let env = Env::new();
    let (llm, tts) = spawn_providers(vec![llm_reply("  \n\t ")], vec![]).await;
    // A whitespace-only answer is not "empty content" for the client; the node rejects it.
    let message = failed(
        voice::run(
            &llm_client(&llm.url, Some(KEY)),
            &tts_client(&tts.url, Some(KEY)),
            &env.ctx,
            &env.inputs(0),
        )
        .await,
    );
    assert!(
        message == "해설 대본이 비어 있습니다" || message == "LLM 응답이 비어 있습니다",
        "{message}"
    );
    assert_eq!(tts.count(), 0);
    assert!(!env.dir.join("mp3.mp3").exists());
}

#[tokio::test]
async fn failures_name_the_failing_step_and_keep_the_script() {
    let env = Env::new();
    let (llm, tts) = spawn_providers(vec![], vec![]).await;

    // No key: the LLM step fails first and nothing is called.
    let message = failed(
        voice::run(
            &llm_client(&llm.url, None),
            &tts_client(&tts.url, None),
            &env.ctx,
            &env.inputs(0),
        )
        .await,
    );
    assert_eq!(message, "OPENROUTER_API_KEY 가 설정되지 않았습니다");
    assert_eq!((llm.count(), tts.count()), (0, 0));

    // The TTS call fails: the script was generated and saved, the audio step reports the error.
    let (llm, tts) = spawn_providers(
        vec![],
        vec![json_error(400, "Voice 'alloy' is not supported")],
    )
    .await;
    let message = failed(
        voice::run(
            &llm_client(&llm.url, Some(KEY)),
            &tts_client(&tts.url, Some(KEY)),
            &env.ctx,
            &env.inputs(0),
        )
        .await,
    );
    assert_eq!(
        message,
        "TTS API 오류 (HTTP 400): Voice 'alloy' is not supported"
    );
    assert!(env.dir.join("_work/script.txt").exists());
    assert!(!env.dir.join("mp3.mp3").exists());
}

// ------------------------------------------------------------------ full run through the API

struct Harness {
    app: Router,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn harness(pool: PgPool, extractor: &str, base: &str) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-voice-run-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let env: HashMap<&str, String> = HashMap::from([
        ("DATABASE_URL", "unused".to_owned()),
        ("DATA_DIR", data.to_string_lossy().into_owned()),
        ("EXTRACTOR_URL", extractor.to_owned()),
        ("OPENROUTER_BASE_URL", base.to_owned()),
        ("OPENROUTER_API_KEY", KEY.to_owned()),
    ]);
    let config = Config::from_lookup(move |k| env.get(k).cloned()).unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    Harness {
        app: build_app(AppState::new(pool, config, http, Arc::new(runner))),
        data,
    }
}

/// Extractor that returns a Markdown file and two images.
async fn spawn_extractor() -> String {
    async fn handler(Json(body): Json<Value>) -> Response {
        let out = PathBuf::from(body["output_dir"].as_str().unwrap());
        tokio::fs::create_dir_all(out.join("images")).await.unwrap();
        tokio::fs::write(out.join("md.md"), "# Report\n\nRevenue grew.")
            .await
            .unwrap();
        tokio::fs::write(out.join("json.json"), "{}").await.unwrap();
        let images = [out.join("images/0000.jpg"), out.join("images/0001.jpg")];
        for (i, path) in images.iter().enumerate() {
            tokio::fs::write(path, [0xff, 0xd8, i as u8]).await.unwrap();
        }
        Json(json!({"md": out.join("md.md"), "json": out.join("json.json"), "tables": [], "images": images})).into_response()
    }
    let app = Router::new().route("/extract", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
}

async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (
        status,
        headers,
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let (status, _, bytes) = raw(app, builder.body(body).unwrap()).await;
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

async fn upload_bundle(app: &Router) -> String {
    let (_, bundle) = call(app, "POST", "/api/bundles", Some(json!({"name": "b"}))).await;
    let id = bundle["id"].as_str().unwrap().to_owned();
    let boundary = "voice-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"doc.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"%PDF-1.7\nx\n%%EOF\n\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let req = Request::post(format!("/api/bundles/{id}/files"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    assert_eq!(raw(app, req).await.0, StatusCode::CREATED);
    id
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

/// bundle → pick_pdf → extract → voice (md + images from the extract node)
fn voice_graph(bundle: &str) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), false),
            node("v", "voice", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "x", "pdf"),
            edge("e3", "x", "md", "v", "md"),
            edge("e4", "x", "images", "v", "images")
        ]
    })
}

async fn run_graph(app: &Router, graph: Value) -> (String, Value) {
    let (_, wf) = call(app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let wf = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = call(
        app,
        "PUT",
        &format!("/api/workflows/{wf}"),
        Some(json!({"graph": graph})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, run) = call(app, "POST", &format!("/api/workflows/{wf}/runs"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["runId"].as_str().unwrap().to_owned();
    for _ in 0..400 {
        let (_, detail) = call(app, "GET", &format!("/api/runs/{run_id}"), None).await;
        if detail["status"] != "running" {
            return (run_id, detail);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run did not finish");
}

fn node_of<'a>(detail: &'a Value, id: &str) -> &'a Value {
    detail["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["nodeId"] == id)
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn extract_to_voice_run_produces_a_downloadable_mp3(pool: PgPool) {
    let extractor = spawn_extractor().await;
    let (llm, tts) = spawn_providers(
        vec![llm_reply("보고서를 설명합니다. 매출이 증가했습니다.")],
        vec![],
    )
    .await;
    let h = harness(pool, &extractor, &llm.url);
    let bundle = upload_bundle(&h.app).await;

    let (run, detail) = run_graph(&h.app, voice_graph(&bundle)).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");
    assert_eq!(node_of(&detail, "v")["status"], "succeeded");
    assert_eq!(node_of(&detail, "v")["outputs"][0]["socketId"], "mp3");

    let (status, headers, body) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run}/artifacts/v/mp3"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "audio/mpeg");
    let audio = decode(&body);
    assert_eq!(audio.rate, RATE);
    assert!(audio.samples.len() > 10_000);

    // The script is kept next to the node's outputs, and the extracted images were sent along.
    let script =
        std::fs::read_to_string(h.data.join("runs").join(&run).join("v/_work/script.txt")).unwrap();
    assert_eq!(script, "보고서를 설명합니다. 매출이 증가했습니다.");
    assert_eq!(tts.tts_inputs(), [script]);
    assert_eq!(
        llm_content(&llm)
            .iter()
            .filter(|b| b["type"] == "image_url")
            .count(),
        2
    );
    assert!(
        llm_content(&llm).last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("# Report")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tts_failures_surface_without_leaking_the_key(pool: PgPool) {
    let extractor = spawn_extractor().await;
    let (llm, _tts) =
        spawn_providers(vec![], vec![json_error(401, &format!("invalid key {KEY}"))]).await;
    let h = harness(pool, &extractor, &llm.url);
    let bundle = upload_bundle(&h.app).await;

    let (_, detail) = run_graph(&h.app, voice_graph(&bundle)).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(node_of(&detail, "x")["status"], "succeeded");
    let error = node_of(&detail, "v")["error"].as_str().unwrap();
    assert!(
        error.starts_with("TTS API 오류 (HTTP 401): invalid key"),
        "{error}"
    );
    assert!(!detail.to_string().contains(KEY));
}
