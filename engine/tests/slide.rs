use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
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
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs,
};
use dochestra_engine::nodes::EngineRunner;
use dochestra_engine::nodes::slide::spec::{SlideContent, SlideDeck};
use dochestra_engine::nodes::slide::{self, render};
use dochestra_engine::state::AppState;
use dochestra_engine::types::{DataType, NodeKind, ScalarType};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const KEY: &str = "sk-slide-secret";

fn font_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts"))
}

/// A real JPEG (gradient) of the given size.
fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([
            (x * 255 / width.max(1)) as u8,
            (y * 255 / height.max(1)) as u8,
            160,
        ])
    });
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&img)
        .unwrap();
    bytes
}

// ------------------------------------------------------------------ PDF inspection (lopdf)

struct PdfInfo {
    pages: usize,
    font_files: usize,
    images: usize,
    media_boxes: Vec<Vec<f32>>,
    size: usize,
}

fn inspect(bytes: &[u8]) -> PdfInfo {
    let doc = lopdf::Document::load_mem(bytes).expect("output must be a readable PDF");
    let mut font_files = 0;
    let mut images = 0;
    for object in doc.objects.values() {
        let dict = match object {
            lopdf::Object::Dictionary(d) => d,
            lopdf::Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if dict.has(b"FontFile2") {
            font_files += 1;
        }
        let is_image = dict
            .get(b"Subtype")
            .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Image"))
            && dict.has(b"Width");
        if is_image {
            images += 1;
        }
    }
    let media_boxes = doc
        .get_pages()
        .values()
        .map(|id| {
            let page = doc.get_dictionary(*id).unwrap();
            page.get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_float().unwrap())
                .collect()
        })
        .collect();
    PdfInfo {
        pages: doc.get_pages().len(),
        font_files,
        images,
        media_boxes,
        size: bytes.len(),
    }
}

fn maybe_dump(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("SLIDE_DUMP_DIR") {
        std::fs::write(PathBuf::from(dir).join(name), bytes).unwrap();
    }
}

fn korean_deck(slides: usize) -> SlideDeck {
    SlideDeck {
        title: "2026년 연간 매출 보고서".to_owned(),
        subtitle: Some("영업본부 · 하반기 실적과 전망".to_owned()),
        slides: (0..slides)
            .map(|i| SlideContent {
                title: format!("{}. 핵심 지표와 시사점", i + 1),
                bullets: vec![
                    "매출은 전년 대비 12.4% 증가한 3,420억 원을 기록했습니다".to_owned(),
                    "해외 시장 비중이 38%로 처음 3분의 1을 넘었습니다".to_owned(),
                    "Operating margin improved to 18.2% (+1.6pp YoY)".to_owned(),
                ],
            })
            .collect(),
    }
}

// ------------------------------------------------------------------ renderer

#[test]
fn deck_with_image_has_cover_slides_and_closing_image_page() {
    let deck = korean_deck(4);
    let bytes = render::render_deck(&deck, Some(&jpeg(640, 360)), &font_dir()).unwrap();
    maybe_dump("with_image.pdf", &bytes);
    let info = inspect(&bytes);

    assert_eq!(info.pages, 1 + 4 + 1);
    assert!(
        info.font_files >= 2,
        "Regular and Bold must both be embedded ({})",
        info.font_files
    );
    assert!(
        info.images >= 1,
        "the representative image must be an XObject"
    );
    for media_box in &info.media_boxes {
        assert_eq!(media_box.len(), 4);
        assert!(
            (media_box[2] - 720.0).abs() < 1.0 && (media_box[3] - 405.0).abs() < 1.0,
            "{media_box:?}"
        );
    }
    assert!(info.size < 10 * 1024 * 1024, "PDF is {} bytes", info.size);
}

#[test]
fn deck_without_image_has_no_closing_page() {
    let deck = korean_deck(3);
    let bytes = render::render_deck(&deck, None, &font_dir()).unwrap();
    maybe_dump("no_image.pdf", &bytes);
    let info = inspect(&bytes);
    assert_eq!(info.pages, 1 + 3);
    assert_eq!(info.images, 0);
    assert!(info.font_files >= 2);
}

#[test]
fn an_undecodable_image_is_skipped_not_fatal() {
    let bytes =
        render::render_deck(&korean_deck(2), Some(b"definitely not a jpeg"), &font_dir()).unwrap();
    let info = inspect(&bytes);
    assert_eq!(info.pages, 1 + 2);
    assert_eq!(info.images, 0);
}

#[test]
fn maximum_content_and_awkward_text_still_render() {
    // 15 slides x 7 bullets of 120 chars: needs shrinking and cutting, must not fail.
    let long = "긴문장없이띄어쓰기가전혀없는한국어문자열입니다".repeat(6);
    let deck = SlideDeck {
        title: "가".repeat(120),
        subtitle: Some("나".repeat(120)),
        slides: (0..15)
            .map(|i| SlideContent {
                title: format!("{i} {}", "제목".repeat(50)),
                bullets: (0..7)
                    .map(|b| format!("{b} {}", long.chars().take(120).collect::<String>()))
                    .collect(),
            })
            .collect(),
    };
    let bytes = render::render_deck(&deck, Some(&jpeg(1200, 300)), &font_dir()).unwrap();
    maybe_dump("max.pdf", &bytes);
    assert_eq!(inspect(&bytes).pages, 1 + 15 + 1);

    // Characters the font lacks (emoji), control characters and tabs are tolerated.
    let odd = SlideDeck {
        title: "Emoji 🚀 title\twith tab".to_owned(),
        subtitle: None,
        slides: vec![SlideContent {
            title: "Mixed ✅ ‑ symbols".to_owned(),
            bullets: vec!["line\nbreak 🎉 inside".to_owned()],
        }],
    };
    assert_eq!(
        inspect(&render::render_deck(&odd, None, &font_dir()).unwrap()).pages,
        2
    );
}

#[test]
fn a_missing_font_directory_is_a_readable_error() {
    let err =
        render::render_deck(&korean_deck(1), None, &PathBuf::from("/no/such/fonts")).unwrap_err();
    assert!(
        err.to_string().starts_with("폰트를 읽을 수 없습니다"),
        "{err}"
    );
}

// ------------------------------------------------------------------ mock Anthropic

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Value,
}

fn text_reply(text: &str) -> Reply {
    Reply {
        status: 200,
        body: json!({"content": [{"type": "text", "text": text}], "stop_reason": "end_turn"}),
    }
}

fn error_reply(status: u16, message: &str) -> Reply {
    Reply {
        status,
        body: json!({"error": {"message": message}}),
    }
}

struct Recorded {
    body: Value,
}

#[derive(Clone)]
struct Shared {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    default: Reply,
}

struct MockLlm {
    url: String,
    shared: Shared,
}

impl MockLlm {
    fn count(&self) -> usize {
        self.shared.requests.lock().unwrap().len()
    }
    fn body(&self, i: usize) -> Value {
        self.shared.requests.lock().unwrap()[i].body.clone()
    }
    fn blocks(&self, i: usize) -> Vec<Value> {
        self.body(i)["messages"][0]["content"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn has_image(&self, i: usize) -> bool {
        self.blocks(i).iter().any(|b| b["type"] == "image")
    }
    fn user_text(&self, i: usize) -> String {
        self.blocks(i)
            .iter()
            .filter(|b| b["type"] == "text")
            .map(|b| b["text"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn deck_answer(slides: usize) -> String {
    let slides: Vec<Value> = (0..slides)
        .map(|i| json!({"title": format!("슬라이드 {i}"), "bullets": ["첫째 요점", "둘째 요점"]}))
        .collect();
    json!({"title": "문서 제목", "subtitle": "부제", "slides": slides}).to_string()
}

async fn spawn_llm(scripted: Vec<Reply>) -> MockLlm {
    let shared = Shared {
        replies: Arc::new(Mutex::new(scripted.into())),
        requests: Arc::new(Mutex::new(Vec::new())),
        default: text_reply(&deck_answer(3)),
    };
    async fn handler(
        State(shared): State<Shared>,
        _headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        shared.requests.lock().unwrap().push(Recorded { body });
        let reply = shared
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| shared.default.clone());
        (
            StatusCode::from_u16(reply.status).unwrap(),
            Json(reply.body),
        )
            .into_response()
    }
    let app = Router::new()
        .route("/v1/messages", post(handler))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    MockLlm { url, shared }
}

fn llm_client(url: &str, key: Option<&str>) -> LlmClient {
    LlmClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "claude-opus-5-5",
        "high",
    )
    .with_backoff_base(Duration::from_millis(5))
}

// ------------------------------------------------------------------ node (direct)

struct Env {
    dir: PathBuf,
    ctx: NodeContext,
}

impl Env {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("dochestra-slide-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = NodeContext {
            run_id: Uuid::new_v4(),
            node_id: "s".to_owned(),
            kind: NodeKind::Slide,
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

    fn inputs(&self, image: Option<&[u8]>) -> NodeInputs {
        let mut inputs = NodeInputs::from([(
            "md".to_owned(),
            Artifact {
                data_type: DataType::single(ScalarType::Md),
                paths: vec![
                    self.file("doc.md", "# 연간 보고서\n\n매출이 증가했습니다.".as_bytes()),
                ],
            },
        )]);
        if let Some(bytes) = image {
            inputs.insert(
                "image".to_owned(),
                Artifact {
                    data_type: DataType::single(ScalarType::Jpg),
                    paths: vec![self.file("pic.jpg", bytes)],
                },
            );
        }
        inputs
    }

    async fn run(&self, llm: &LlmClient, inputs: &NodeInputs) -> Result<NodeOutputs, NodeError> {
        slide::run(llm, &font_dir(), &self.ctx, inputs).await
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn failed(result: Result<NodeOutputs, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(message)) => message,
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn node_renders_the_llm_outline_into_a_pdf() {
    let env = Env::new();
    let mock = spawn_llm(vec![text_reply(&format!(
        "구성은 아래와 같습니다.\n```json\n{}\n```",
        deck_answer(3)
    ))])
    .await;
    let out = env
        .run(
            &llm_client(&mock.url, Some(KEY)),
            &env.inputs(Some(&jpeg(400, 300))),
        )
        .await
        .unwrap();

    assert_eq!(out["pdf"].data_type, DataType::single(ScalarType::Pdf));
    assert_eq!(out["pdf"].paths, vec![env.dir.join("pdf.pdf")]);
    let bytes = std::fs::read(&out["pdf"].paths[0]).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let info = inspect(&bytes);
    assert_eq!(info.pages, 1 + 3 + 1);
    assert!(info.font_files >= 2 && info.images >= 1);

    // The normalized outline is kept next to the node's work files.
    let saved: SlideDeck =
        serde_json::from_slice(&std::fs::read(env.dir.join("_work/slides.json")).unwrap()).unwrap();
    assert_eq!(saved.title, "문서 제목");
    assert_eq!(saved.slides.len(), 3);

    // Request: image block first, JSON-only system prompt, whole document in the prompt.
    assert_eq!(mock.count(), 1);
    assert_eq!(mock.blocks(0)[0]["type"], "image");
    assert!(
        mock.body(0)["system"]
            .as_str()
            .unwrap()
            .contains("ONLY one JSON object")
    );
    assert!(mock.user_text(0).contains("<document>\n# 연간 보고서"));
    assert!(mock.user_text(0).contains("대표 이미지"));
}

#[tokio::test]
async fn node_without_image_makes_one_page_less_and_sends_no_image() {
    let env = Env::new();
    let mock = spawn_llm(vec![]).await;
    let out = env
        .run(&llm_client(&mock.url, Some(KEY)), &env.inputs(None))
        .await
        .unwrap();
    assert_eq!(
        inspect(&std::fs::read(&out["pdf"].paths[0]).unwrap()).pages,
        1 + 3
    );
    assert!(!mock.has_image(0));
}

#[tokio::test]
async fn an_image_over_5_mib_is_left_out_but_the_deck_is_still_built() {
    let env = Env::new();
    let huge = vec![0u8; 5 * 1024 * 1024 + 1];
    let mock = spawn_llm(vec![]).await;
    let out = env
        .run(&llm_client(&mock.url, Some(KEY)), &env.inputs(Some(&huge)))
        .await
        .unwrap();
    assert!(!mock.has_image(0));
    let info = inspect(&std::fs::read(&out["pdf"].paths[0]).unwrap());
    assert_eq!(info.pages, 1 + 3);
    assert_eq!(info.images, 0);
}

#[tokio::test]
async fn outline_limits_are_enforced_by_truncation() {
    let env = Env::new();
    let mock = spawn_llm(vec![text_reply(&deck_answer(20))]).await;
    let out = env
        .run(&llm_client(&mock.url, Some(KEY)), &env.inputs(None))
        .await
        .unwrap();
    // 20 requested slides are cut to 15 (+ cover).
    assert_eq!(
        inspect(&std::fs::read(&out["pdf"].paths[0]).unwrap()).pages,
        1 + 15
    );
}

#[tokio::test]
async fn an_unusable_answer_is_retried_once_with_the_reason() {
    let env = Env::new();
    let mock = spawn_llm(vec![
        text_reply("죄송하지만 JSON 을 만들 수 없습니다"),
        text_reply(&deck_answer(2)),
    ])
    .await;
    let out = env
        .run(&llm_client(&mock.url, Some(KEY)), &env.inputs(None))
        .await
        .unwrap();
    assert_eq!(mock.count(), 2);
    assert!(!mock.user_text(0).contains("이전 응답은 해석에 실패"));
    let retry = mock.user_text(1);
    assert!(
        retry.contains("이전 응답은 해석에 실패했습니다: 응답에서 JSON 객체를 찾을 수 없습니다"),
        "{retry}"
    );
    assert_eq!(
        inspect(&std::fs::read(&out["pdf"].paths[0]).unwrap()).pages,
        1 + 2
    );
}

#[tokio::test]
async fn two_unusable_answers_fail_the_node() {
    let env = Env::new();
    let empty_deck = r#"{"title":"T","slides":[]}"#;
    let mock = spawn_llm(vec![text_reply("not json"), text_reply(empty_deck)]).await;
    let message = failed(
        env.run(&llm_client(&mock.url, Some(KEY)), &env.inputs(None))
            .await,
    );
    assert_eq!(
        message,
        "슬라이드 구성을 해석하지 못했습니다: slides 가 비어 있습니다 (1~15개 필요)"
    );
    assert_eq!(mock.count(), 2);
    assert!(!env.dir.join("pdf.pdf").exists());
}

#[tokio::test]
async fn llm_and_font_failures_are_reported() {
    let env = Env::new();
    // API errors are not "unusable answers": no second outline request.
    let mock = spawn_llm(vec![error_reply(400, "bad request")]).await;
    let message = failed(
        env.run(&llm_client(&mock.url, Some(KEY)), &env.inputs(None))
            .await,
    );
    assert_eq!(message, "LLM API 오류 (HTTP 400): bad request");
    assert_eq!(mock.count(), 1);

    let mock = spawn_llm(vec![]).await;
    let message = failed(
        env.run(&llm_client(&mock.url, None), &env.inputs(None))
            .await,
    );
    assert_eq!(message, "LLM_API_KEY 가 설정되지 않았습니다");

    let message = failed(
        slide::run(
            &llm_client(&mock.url, Some(KEY)),
            &PathBuf::from("/no/fonts"),
            &env.ctx,
            &env.inputs(None),
        )
        .await,
    );
    assert!(
        message.starts_with("슬라이드 PDF 를 만들 수 없습니다: 폰트를 읽을 수 없습니다"),
        "{message}"
    );
}

#[tokio::test]
async fn a_document_over_the_request_limit_fails_without_a_call() {
    let env = Env::new();
    let mut inputs = env.inputs(None);
    let huge = "a".repeat(31 * 1024 * 1024);
    inputs.insert(
        "md".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Md),
            paths: vec![env.file("huge.md", huge.as_bytes())],
        },
    );
    let mock = spawn_llm(vec![]).await;
    let message = failed(env.run(&llm_client(&mock.url, Some(KEY)), &inputs).await);
    assert_eq!(message, "문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다");
    assert_eq!(mock.count(), 0);
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

fn harness(pool: PgPool, extractor: &str, llm: &str) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-slide-run-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let env: HashMap<&str, String> = HashMap::from([
        ("DATABASE_URL", "unused".to_owned()),
        ("DATA_DIR", data.to_string_lossy().into_owned()),
        ("EXTRACTOR_URL", extractor.to_owned()),
        ("LLM_BASE_URL", llm.to_owned()),
        ("LLM_API_KEY", KEY.to_owned()),
        ("SLIDE_FONT_DIR", font_dir().to_string_lossy().into_owned()),
    ]);
    let config = Config::from_lookup(move |k| env.get(k).cloned()).unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    Harness {
        app: build_app(AppState::new(pool, config, http, Arc::new(runner))),
        data,
    }
}

/// Extractor answering with a Markdown file and one real JPEG image.
async fn spawn_extractor() -> String {
    async fn handler(Json(body): Json<Value>) -> Response {
        let out = PathBuf::from(body["output_dir"].as_str().unwrap());
        tokio::fs::create_dir_all(out.join("images")).await.unwrap();
        tokio::fs::write(out.join("md.md"), "# 연간 보고서\n\n매출이 증가했습니다.")
            .await
            .unwrap();
        tokio::fs::write(out.join("json.json"), "{}").await.unwrap();
        let image = out.join("images/0000.jpg");
        tokio::fs::write(&image, jpeg(320, 200)).await.unwrap();
        Json(json!({"md": out.join("md.md"), "json": out.join("json.json"), "tables": [], "images": [image]})).into_response()
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
    let boundary = "slide-boundary";
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

/// bundle → pick_pdf → extract → {extract.md → slide.md, extract.images → pick_jpg → slide.image}
fn slide_graph(bundle: &str) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), false),
            node("j", "pick_jpg", json!({"index": 0}), false),
            node("s", "slide", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "x", "pdf"),
            edge("e3", "x", "md", "s", "md"),
            edge("e4", "x", "images", "j", "images"),
            edge("e5", "j", "image", "s", "image")
        ]
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn extract_pick_jpg_slide_run_produces_a_downloadable_pdf(pool: PgPool) {
    let extractor = spawn_extractor().await;
    let llm = spawn_llm(vec![text_reply(&deck_answer(4))]).await;
    let h = harness(pool, &extractor, &llm.url);
    let bundle = upload_bundle(&h.app).await;

    let (_, wf) = call(&h.app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let wf = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = call(
        &h.app,
        "PUT",
        &format!("/api/workflows/{wf}"),
        Some(json!({"graph": slide_graph(&bundle)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, run) = call(&h.app, "POST", &format!("/api/workflows/{wf}/runs"), None).await;
    let run_id = run["runId"].as_str().unwrap().to_owned();
    let mut detail = Value::Null;
    for _ in 0..400 {
        let (_, d) = call(&h.app, "GET", &format!("/api/runs/{run_id}"), None).await;
        if d["status"] != "running" {
            detail = d;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(detail["status"], "succeeded", "{detail}");

    let (status, headers, bytes) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run_id}/artifacts/s/pdf"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    maybe_dump("run.pdf", &bytes);
    let info = inspect(&bytes);
    assert_eq!(info.pages, 1 + 4 + 1);
    assert!(info.images >= 1 && info.font_files >= 2);
    // The extracted image reached the LLM as the representative picture.
    assert!(llm.has_image(0));
}
