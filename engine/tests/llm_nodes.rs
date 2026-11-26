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
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dochestra_engine::build_app;
use dochestra_engine::clients::llm::{
    LlmClient, LlmError, LlmRequest, MAX_REQUEST_BYTES, UserBlock,
};
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs,
};
use dochestra_engine::nodes::{EngineRunner, analyze, summary, translate};
use dochestra_engine::state::AppState;
use dochestra_engine::types::{DataType, NodeKind, ScalarType};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

// ------------------------------------------------------------------ mock OpenRouter (chat/completions)

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

fn ok(text: &str) -> Reply {
    Reply {
        status: 200,
        body: json!({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]})
            .to_string(),
        headers: vec![],
    }
}

fn raw(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: body.to_string(),
        headers: vec![],
    }
}

fn error(status: u16, message: &str) -> Reply {
    raw(
        status,
        json!({"error": {"message": message, "code": status}}),
    )
}

struct Recorded {
    headers: HashMap<String, String>,
    body: Value,
}

#[derive(Clone)]
struct Shared {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
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

    fn header(&self, i: usize, name: &str) -> Option<String> {
        self.shared.requests.lock().unwrap()[i]
            .headers
            .get(name)
            .cloned()
    }

    /// Text of the last text block of request `i`.
    fn user_text(&self, i: usize) -> String {
        let body = self.body(i);
        body["messages"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "text")
            .map(|b| b["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join("")
    }

    fn image_blocks(&self, i: usize) -> Vec<Value> {
        self.body(i)["messages"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "image_url")
            .cloned()
            .collect()
    }
}

async fn spawn_llm(replies: Vec<Reply>) -> MockLlm {
    let shared = Shared {
        replies: Arc::new(Mutex::new(replies.into())),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    // Large image requests must reach the handler (axum's default limit is 2 MiB).
    let app = Router::new()
        .route("/chat/completions", post(llm_handler))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    MockLlm { url, shared }
}

async fn llm_handler(
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
    shared.requests.lock().unwrap().push(Recorded {
        headers: recorded_headers,
        body,
    });
    let reply = shared
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| error(418, "no scripted reply"));
    let mut resp = (StatusCode::from_u16(reply.status).unwrap(), reply.body).into_response();
    resp.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    for (name, value) in reply.headers {
        resp.headers_mut().insert(name, value.parse().unwrap());
    }
    resp
}

const KEY: &str = "sk-test-key-123";

/// Base64 payload of an `image_url` content part (`data:image/jpeg;base64,...`).
fn image_b64(block: &Value) -> String {
    block["image_url"]["url"]
        .as_str()
        .and_then(|u| u.strip_prefix("data:image/jpeg;base64,"))
        .expect("jpeg data URL")
        .to_owned()
}

fn client(url: &str, key: Option<&str>) -> LlmClient {
    LlmClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "meta/muse-ember",
        "medium",
    )
    .with_backoff_base(Duration::from_millis(5))
}

fn simple(text: &str) -> LlmRequest {
    LlmRequest {
        system: "be brief".to_owned(),
        user_content: vec![UserBlock::Text(text.to_owned())],
    }
}

// ------------------------------------------------------------------ client behaviour

#[tokio::test]
async fn request_has_the_documented_shape() {
    let mock = spawn_llm(vec![ok("answer")]).await;
    let req = LlmRequest {
        system: "system prompt".to_owned(),
        user_content: vec![
            UserBlock::Text("first text".to_owned()),
            UserBlock::ImageJpeg(vec![1, 2, 3]),
            UserBlock::Text("second text".to_owned()),
        ],
    };
    assert_eq!(
        client(&mock.url, Some(KEY)).complete(req).await.unwrap(),
        "answer"
    );
    assert_eq!(mock.count(), 1);

    assert_eq!(
        mock.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );
    assert_eq!(mock.header(0, "x-title").as_deref(), Some("Dochestra"));
    assert!(
        mock.header(0, "content-type")
            .unwrap()
            .starts_with("application/json")
    );
    for retired in ["x-api-key", "anthropic-version", "anthropic-beta"] {
        assert!(
            mock.header(0, retired).is_none(),
            "{retired} must not be sent"
        );
    }

    let body = mock.body(0);
    assert_eq!(body["model"], "meta/muse-ember");
    assert_eq!(body["max_tokens"], 16000);
    assert_eq!(body["reasoning"], json!({"effort": "medium"}));
    for retired in [
        "system",
        "output_config",
        "fallbacks",
        "thinking",
        "temperature",
        "top_p",
        "top_k",
        "tool_choice",
        "stream",
    ] {
        assert!(body.get(retired).is_none(), "{retired} must not be sent");
    }
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0],
        json!({"role": "system", "content": "system prompt"})
    );
    assert_eq!(messages[1]["role"], "user");
    // Image parts come first, then the text parts in their original order.
    assert_eq!(
        messages[1]["content"],
        json!([
            {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,AQID"}},
            {"type": "text", "text": "first text"},
            {"type": "text", "text": "second text"},
        ])
    );
}

#[tokio::test]
async fn the_request_goes_to_chat_completions_under_the_base_url() {
    // The client appends `/chat/completions` to whatever base URL it is given (here with a
    // path prefix and a trailing slash).
    let mock = spawn_llm(vec![ok("fine")]).await;
    let c = client(&format!("{}/", mock.url), Some(KEY));
    assert_eq!(c.complete(simple("x")).await.unwrap(), "fine");
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn array_content_parts_are_joined() {
    let body = json!({"choices": [{"message": {"role": "assistant", "content": [
        {"type": "text", "text": "Hello, "},
        {"type": "reasoning", "text": "hidden"},
        {"type": "text", "text": "world"}
    ]}, "finish_reason": "stop"}]});
    let mock = spawn_llm(vec![raw(200, body)]).await;
    assert_eq!(
        client(&mock.url, Some(KEY))
            .complete(simple("hi"))
            .await
            .unwrap(),
        "Hello, world"
    );
}

#[tokio::test]
async fn length_empty_and_refused_answers_are_reported_without_retry() {
    let refusal = raw(
        200,
        json!({"choices": [{"message": {"content": null, "refusal": "I cannot help with that."}, "finish_reason": "stop"}]}),
    );
    let truncated = raw(
        200,
        json!({"choices": [{"message": {"content": "cut"}, "finish_reason": "length"}]}),
    );
    // A reasoning model that spent the whole budget on thinking: content null, finish length.
    let all_reasoning = raw(
        200,
        json!({"choices": [{"message": {"content": null}, "finish_reason": "length"}]}),
    );
    let empty = raw(
        200,
        json!({"choices": [{"message": {"content": ""}, "finish_reason": "stop"}]}),
    );
    let null = raw(
        200,
        json!({"choices": [{"message": {"content": null}, "finish_reason": "stop"}]}),
    );
    let mock = spawn_llm(vec![refusal, truncated, all_reasoning, empty, null]).await;
    let c = client(&mock.url, Some(KEY));

    let err = c.complete(simple("x")).await.unwrap_err();
    assert_eq!(err, LlmError::Refusal);
    assert_eq!(err.to_string(), "LLM 이 요청을 거절했습니다");
    for _ in 0..2 {
        let err = c.complete(simple("x")).await.unwrap_err();
        assert_eq!(err, LlmError::MaxTokens);
        assert_eq!(err.to_string(), "LLM 출력이 최대 길이에 도달했습니다");
    }
    for _ in 0..2 {
        let err = c.complete(simple("x")).await.unwrap_err();
        assert_eq!(err, LlmError::Empty);
        assert_eq!(err.to_string(), "LLM 응답이 비어 있습니다");
    }
    assert_eq!(mock.count(), 5); // none of them is retried
}

#[tokio::test]
async fn errors_reported_inside_a_200_body_are_handled_like_http_errors() {
    // OpenRouter can answer 200 with a body-level error for upstream failures.
    let transient = raw(
        200,
        json!({"error": {"message": "Provider returned error", "code": 429}}),
    );
    let mock = spawn_llm(vec![transient, ok("recovered")]).await;
    assert_eq!(
        client(&mock.url, Some(KEY))
            .complete(simple("x"))
            .await
            .unwrap(),
        "recovered"
    );
    assert_eq!(mock.count(), 2);

    let fatal = raw(
        200,
        json!({"error": {"message": "invalid model", "code": 400}}),
    );
    let mock = spawn_llm(vec![fatal]).await;
    let err = client(&mock.url, Some(KEY))
        .complete(simple("x"))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        LlmError::Api {
            status: 400,
            message: "invalid model".to_owned()
        }
    );
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn transient_errors_are_retried() {
    // 429 with retry-after: 0, then success.
    let mut rate_limited = error(429, "slow down");
    rate_limited.headers.push(("retry-after", "0"));
    let mock = spawn_llm(vec![rate_limited, ok("done")]).await;
    assert_eq!(
        client(&mock.url, Some(KEY))
            .complete(simple("x"))
            .await
            .unwrap(),
        "done"
    );
    assert_eq!(mock.count(), 2);

    // Three failures (503, 500, 502), then success on the 4th (= last allowed) attempt.
    let mock = spawn_llm(vec![
        error(503, "a"),
        error(500, "b"),
        error(502, "c"),
        ok("finally"),
    ])
    .await;
    assert_eq!(
        client(&mock.url, Some(KEY))
            .complete(simple("x"))
            .await
            .unwrap(),
        "finally"
    );
    assert_eq!(mock.count(), 4);

    // Persistent failure gives up after 1 + 3 attempts with the last error.
    let mock = spawn_llm(vec![
        error(502, "1"),
        error(502, "2"),
        error(502, "3"),
        error(502, "last"),
    ])
    .await;
    let err = client(&mock.url, Some(KEY))
        .complete(simple("x"))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        LlmError::Api {
            status: 502,
            message: "last".to_owned()
        }
    );
    assert_eq!(mock.count(), 4);
}

#[tokio::test]
async fn client_errors_fail_immediately_with_the_api_message() {
    // 529 (Anthropic's "overloaded") is not part of the OpenRouter retry set either.
    for status in [400, 401, 402, 403, 404, 413, 529] {
        let mock = spawn_llm(vec![error(status, "model not found")]).await;
        let err = client(&mock.url, Some(KEY))
            .complete(simple("x"))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            LlmError::Api {
                status,
                message: "model not found".to_owned()
            }
        );
        assert_eq!(
            err.to_string(),
            format!("LLM API 오류 (HTTP {status}): model not found")
        );
        assert_eq!(mock.count(), 1, "HTTP {status} must not be retried");
    }
}

#[tokio::test]
async fn connection_failures_are_retried_then_reported() {
    // Nothing listens on port 9.
    let err = client("http://127.0.0.1:9", Some(KEY))
        .complete(simple("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Transport(_)), "{err:?}");
    assert!(
        err.to_string()
            .starts_with("LLM API 에 연결할 수 없습니다: ")
    );
}

#[tokio::test]
async fn missing_key_fails_without_any_http_call() {
    let mock = spawn_llm(vec![ok("never")]).await;
    for key in [None, Some("")] {
        let err = client(&mock.url, key)
            .complete(simple("x"))
            .await
            .unwrap_err();
        assert_eq!(err, LlmError::MissingApiKey);
        assert_eq!(err.to_string(), "OPENROUTER_API_KEY 가 설정되지 않았습니다");
    }
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn the_api_key_never_appears_in_errors_or_debug_output() {
    // An upstream that echoes the key back must not leak it through our error.
    let mock = spawn_llm(vec![error(401, &format!("invalid key: {KEY}"))]).await;
    let c = client(&mock.url, Some(KEY));
    let err = c.complete(simple("x")).await.unwrap_err();
    assert!(!err.to_string().contains(KEY), "{err}");
    assert!(!format!("{err:?}").contains(KEY));
    assert!(!format!("{c:?}").contains(KEY));
}

// ------------------------------------------------------------------ nodes (direct)

struct Env {
    dir: PathBuf,
    ctx: NodeContext,
}

impl Env {
    fn new(kind: NodeKind) -> Self {
        let dir = std::env::temp_dir().join(format!("dochestra-llm-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = NodeContext {
            run_id: Uuid::new_v4(),
            node_id: "n".to_owned(),
            kind,
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

    fn output_text(&self, outputs: &NodeOutputs) -> String {
        std::fs::read_to_string(&outputs["md"].paths[0]).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn input(scalar: ScalarType, list: bool, paths: Vec<PathBuf>) -> Artifact {
    Artifact {
        data_type: if list {
            DataType::list_of(scalar)
        } else {
            DataType::single(scalar)
        },
        paths,
    }
}

fn failed(result: Result<NodeOutputs, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(message)) => message,
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn translate_inputs(env: &Env, md: &str, settings: &str) -> NodeInputs {
    NodeInputs::from([
        (
            "md".to_owned(),
            input(
                ScalarType::Md,
                false,
                vec![env.file("in.md", md.as_bytes())],
            ),
        ),
        (
            "settings".to_owned(),
            input(
                ScalarType::Json,
                false,
                vec![env.file("settings.json", settings.as_bytes())],
            ),
        ),
    ])
}

#[tokio::test]
async fn translate_rejects_invalid_settings_without_calling_the_llm() {
    let env = Env::new(NodeKind::Translate);
    let mock = spawn_llm(vec![ok("x")]).await;
    for settings in [
        "not json",
        "{}",
        r#"{"targetLanguage":""}"#,
        r#"["ko"]"#,
        r#"{"targetLanguage":"ko","glossary":{"a":1}}"#,
    ] {
        let inputs = translate_inputs(&env, "# Hi", settings);
        let message =
            failed(translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs).await);
        assert!(
            message.starts_with("번역 설정 JSON 이 올바르지 않습니다: "),
            "{settings}: {message}"
        );
    }
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn translate_preserves_chunk_order_and_code_blocks() {
    let env = Env::new(NodeKind::Translate);
    // Sizes chosen so that chunk 1 = `a`, chunk 2 = fenced block + `b`, chunk 3 = `c`.
    // The fenced block contains a blank line and must not be split.
    // (Uppercase: lowercase-initial blocks would be re-joined as page-break fragments.)
    let a = "A".repeat(7990);
    let fenced = "```\nline1\n\nline2\n```".to_owned();
    let b = "B".repeat(4000);
    let c = "C".repeat(5000);
    let md = format!("{a}\n\n{fenced}\n\n{b}\n\n{c}");

    let mock = spawn_llm(vec![ok("T1\n"), ok("T2"), ok("T3\n\n")]).await;
    let inputs = translate_inputs(
        &env,
        &md,
        r#"{"targetLanguage":"Korean","glossary":{"widget":"위젯"},"tone":"formal"}"#,
    );
    let out = translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();

    assert_eq!(out["md"].data_type, DataType::single(ScalarType::Md));
    assert_eq!(env.output_text(&out), "T1\n\nT2\n\nT3");
    assert_eq!(mock.count(), 3);
    assert_eq!(mock.user_text(0), a);
    assert_eq!(mock.user_text(2), c);
    let middle = mock.user_text(1);
    assert!(
        middle.starts_with(&fenced),
        "code block must stay whole: {middle:?}"
    );
    // The system prompt carries the settings.
    let system = mock.body(0)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        system.contains("Target language: Korean")
            && system.contains("widget => 위젯")
            && system.contains("formal")
    );
    assert!(system.contains("<!-- image -->"));
}

#[tokio::test]
async fn translate_keeps_headings_with_their_body_across_chunks() {
    let env = Env::new(NodeKind::Translate);
    // Chunk 1 would end with `## Section` (7990 + 2 + 10 <= 8000); the heading must move on.
    let a = "A".repeat(7980);
    let md = format!("{a}\n\n## Section\n\n{}\n\nTail.", "B".repeat(3000));
    let mock = spawn_llm(vec![ok("T1"), ok("T2")]).await;
    let inputs = translate_inputs(&env, &md, r#"{"targetLanguage":"ko"}"#);
    translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(mock.count(), 2);
    assert_eq!(mock.user_text(0), a);
    assert!(
        mock.user_text(1).starts_with("## Section\n\nBBB"),
        "{}",
        &mock.user_text(1)[..40]
    );
}

#[tokio::test]
async fn translate_rejoins_page_break_fragments_before_sending() {
    let env = Env::new(NodeKind::Translate);
    let md = "The fleet controller assigns every pick task to the nearest idle\n\nrobot and re-plans the route. Done.";
    let mock = spawn_llm(vec![ok("번역")]).await;
    let inputs = translate_inputs(&env, md, r#"{"targetLanguage":"ko"}"#);
    translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(mock.count(), 1);
    assert_eq!(
        mock.user_text(0),
        "The fleet controller assigns every pick task to the nearest idle robot and re-plans the route. Done."
    );
}

#[tokio::test]
async fn translate_restores_code_and_unwraps_a_fenced_answer() {
    let env = Env::new(NodeKind::Translate);
    let md = "Run it:\n\n```sh\nls -l\n```";
    // The model translated the code comment-free command and wrapped everything in a fence.
    let mock = spawn_llm(vec![ok(
        "```markdown\n실행하세요:\n\n```sh\nls -la\n```\n```",
    )])
    .await;
    let inputs = translate_inputs(&env, md, r#"{"targetLanguage":"ko"}"#);
    let out = translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(env.output_text(&out), "실행하세요:\n\n```sh\nls -l\n```");
}

#[tokio::test]
async fn translate_of_an_empty_document_makes_no_calls() {
    let env = Env::new(NodeKind::Translate);
    let mock = spawn_llm(vec![]).await;
    let inputs = translate_inputs(&env, "\n\n", r#"{"targetLanguage":"ko"}"#);
    let out = translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(env.output_text(&out), "");
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn llm_failures_become_node_failures() {
    let env = Env::new(NodeKind::Translate);
    let mock = spawn_llm(vec![error(400, "bad request")]).await;
    let inputs = translate_inputs(&env, "# Hi", r#"{"targetLanguage":"ko"}"#);
    let message = failed(translate::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs).await);
    assert_eq!(message, "LLM API 오류 (HTTP 400): bad request");

    let message = failed(translate::run(&client(&mock.url, None), &env.ctx, &inputs).await);
    assert_eq!(message, "OPENROUTER_API_KEY 가 설정되지 않았습니다");
}

#[tokio::test]
async fn analyze_sends_the_image_and_enforces_the_size_limit() {
    let env = Env::new(NodeKind::Analyze);
    let jpg = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
    let inputs = NodeInputs::from([(
        "image".to_owned(),
        input(ScalarType::Jpg, false, vec![env.file("pic.jpg", &jpg)]),
    )]);
    let mock = spawn_llm(vec![ok("## 차트\n설명")]).await;
    let out = analyze::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(env.output_text(&out), "## 차트\n설명");
    let images = mock.image_blocks(0);
    assert_eq!(images.len(), 1);
    assert_eq!(image_b64(&images[0]), STANDARD.encode(&jpg));
    assert_eq!(
        mock.body(0)["messages"][1]["content"][0]["type"],
        "image_url"
    ); // image before text

    // Exactly 5 MiB is fine, one byte more is rejected before any request.
    let exact = vec![0u8; 5 * 1024 * 1024];
    let big = vec![0u8; 5 * 1024 * 1024 + 1];
    let mock = spawn_llm(vec![ok("fine")]).await;
    let inputs = NodeInputs::from([(
        "image".to_owned(),
        input(ScalarType::Jpg, false, vec![env.file("exact.jpg", &exact)]),
    )]);
    assert!(
        analyze::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
            .await
            .is_ok()
    );
    let inputs = NodeInputs::from([(
        "image".to_owned(),
        input(ScalarType::Jpg, false, vec![env.file("big.jpg", &big)]),
    )]);
    let before = mock.count();
    let message = failed(analyze::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs).await);
    assert_eq!(message, "이미지가 너무 큽니다 (최대 5MB)");
    assert_eq!(mock.count(), before);
}

fn summary_inputs(env: &Env, images: Option<Vec<PathBuf>>) -> NodeInputs {
    let mut inputs = NodeInputs::from([(
        "md".to_owned(),
        input(
            ScalarType::Md,
            false,
            vec![env.file("doc.md", "# Doc\n\n본문 내용".as_bytes())],
        ),
    )]);
    if let Some(paths) = images {
        inputs.insert("images".to_owned(), input(ScalarType::Jpg, true, paths));
    }
    inputs
}

fn images(env: &Env, n: usize) -> Vec<PathBuf> {
    (0..n)
        .map(|i| env.file(&format!("img{i:02}.jpg"), &[0xff, 0xd8, i as u8]))
        .collect()
}

#[tokio::test]
async fn summary_works_without_images() {
    for images in [None, Some(vec![])] {
        let env = Env::new(NodeKind::Summary);
        let mock = spawn_llm(vec![ok("## 핵심 요점\n- a")]).await;
        let out = summary::run(
            &client(&mock.url, Some(KEY)),
            &env.ctx,
            &summary_inputs(&env, images),
        )
        .await
        .unwrap();
        assert_eq!(env.output_text(&out), "## 핵심 요점\n- a");
        assert!(mock.image_blocks(0).is_empty());
        let text = mock.user_text(0);
        assert!(
            text.contains("<document>\n# Doc\n\n본문 내용\n</document>"),
            "{text}"
        );
        assert!(!text.contains("첨부된 이미지"));
    }
}

#[tokio::test]
async fn summary_sends_images_before_the_document_in_order() {
    let env = Env::new(NodeKind::Summary);
    let paths = images(&env, 3);
    let mock = spawn_llm(vec![ok("요약")]).await;
    summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, Some(paths)),
    )
    .await
    .unwrap();
    let content = mock.body(0)["messages"][1]["content"].clone();
    let types: Vec<&str> = content
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["image_url", "image_url", "image_url", "text"]);
    for (i, block) in mock.image_blocks(0).iter().enumerate() {
        assert_eq!(image_b64(block), STANDARD.encode([0xff, 0xd8, i as u8]));
    }
    let text = mock.user_text(0);
    assert!(text.contains("첨부된 이미지 3개"));
    // The model is told how the images map onto the document.
    assert!(text.contains("<!-- image -->") && text.contains("순서대로"));
}

#[tokio::test]
async fn summary_prompt_demands_korean_brevity_and_no_invented_numbers() {
    let env = Env::new(NodeKind::Summary);
    let mock = spawn_llm(vec![ok("요약")]).await;
    summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, None),
    )
    .await
    .unwrap();
    let system = mock.body(0)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(system.contains("반드시 한국어") && system.contains("hours"));
    let text = mock.user_text(0);
    assert!(text.contains("## 핵심 요점"));
    assert!(text.contains("원문보다 분명히 짧아야"), "{text}");
    assert!(text.contains("문서에 없는 수치는 쓰지 않습니다"));
    // Every section keeps its own heading, and the key points are not repeated per section.
    assert!(text.contains("소제목을 따로 둡니다") && text.contains("되풀이하지 말고"));
    assert!(!text.contains("첨부된 이미지"));
}

#[tokio::test]
async fn summary_limits_images_to_twenty_and_notes_the_exclusions() {
    let env = Env::new(NodeKind::Summary);
    let mock = spawn_llm(vec![ok("요약 본문\n")]).await;
    let out = summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, Some(images(&env, 21))),
    )
    .await
    .unwrap();
    assert_eq!(mock.image_blocks(0).len(), 20);
    assert_eq!(
        env.output_text(&out),
        "요약 본문\n\n※ 이미지 1개는 크기/개수 제한으로 제외됨"
    );

    // An oversized image is skipped (and counted) while the others are kept.
    let env = Env::new(NodeKind::Summary);
    let mut paths = images(&env, 2);
    paths.insert(1, env.file("huge.jpg", &vec![0u8; 5 * 1024 * 1024 + 1]));
    let mock = spawn_llm(vec![ok("요약")]).await;
    let out = summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, Some(paths)),
    )
    .await
    .unwrap();
    assert_eq!(mock.image_blocks(0).len(), 2);
    assert!(
        env.output_text(&out)
            .ends_with("※ 이미지 1개는 크기/개수 제한으로 제외됨")
    );
    // Exactly 20 images: nothing excluded, no note.
    let env = Env::new(NodeKind::Summary);
    let mock = spawn_llm(vec![ok("요약")]).await;
    let out = summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, Some(images(&env, 20))),
    )
    .await
    .unwrap();
    assert_eq!(env.output_text(&out), "요약");
}

#[tokio::test]
async fn summary_sends_the_whole_document_untruncated() {
    let env = Env::new(NodeKind::Summary);
    let long = "문장입니다. ".repeat(20_000); // far larger than a translation chunk
    let inputs = NodeInputs::from([(
        "md".to_owned(),
        input(
            ScalarType::Md,
            false,
            vec![env.file("long.md", long.as_bytes())],
        ),
    )]);
    let mock = spawn_llm(vec![ok("요약")]).await;
    summary::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert!(mock.user_text(0).contains(&long));
}

// ------------------------------------------------------------------ request size limit (30 MiB)

fn content_length(mock: &MockLlm, i: usize) -> usize {
    mock.header(i, "content-length")
        .expect("content-length header")
        .parse()
        .unwrap()
}

#[tokio::test]
async fn oversized_requests_are_rejected_before_any_http_call() {
    let mock = spawn_llm(vec![ok("never")]).await;
    let err = client(&mock.url, Some(KEY))
        .complete(simple(&"x".repeat(MAX_REQUEST_BYTES)))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::RequestTooLarge);
    assert_eq!(
        err.to_string(),
        "문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다"
    );
    assert_eq!(mock.count(), 0);

    // A request just under the limit is sent.
    let under = "x".repeat(MAX_REQUEST_BYTES - 5 * 1024);
    let mock = spawn_llm(vec![ok("sent")]).await;
    assert_eq!(
        client(&mock.url, Some(KEY))
            .complete(simple(&under))
            .await
            .unwrap(),
        "sent"
    );
    assert!(content_length(&mock, 0) <= MAX_REQUEST_BYTES);
}

#[tokio::test]
async fn summary_keeps_images_only_while_the_request_fits() {
    let env = Env::new(NodeKind::Summary);
    // 12 images of 4 MiB: base64 makes each ~5.6 MB, so only the first five fit in 30 MiB.
    let paths: Vec<PathBuf> = (0..12u8)
        .map(|i| env.file(&format!("big{i:02}.jpg"), &vec![i + 1; 4 * 1024 * 1024]))
        .collect();
    let mock = spawn_llm(vec![ok("요약")]).await;
    let out = summary::run(
        &client(&mock.url, Some(KEY)),
        &env.ctx,
        &summary_inputs(&env, Some(paths)),
    )
    .await
    .unwrap();

    let blocks = mock.image_blocks(0);
    assert_eq!(blocks.len(), 5);
    assert!(
        content_length(&mock, 0) <= MAX_REQUEST_BYTES,
        "{}",
        content_length(&mock, 0)
    );
    // The first images (in order) are the ones kept.
    for (i, block) in blocks.iter().enumerate() {
        let prefix = STANDARD.encode([i as u8 + 1; 3]);
        assert!(image_b64(block).starts_with(&prefix), "image {i}");
    }
    assert_eq!(
        env.output_text(&out),
        "요약\n\n※ 이미지 7개는 크기/개수 제한으로 제외됨"
    );
}

#[tokio::test]
async fn summary_counts_the_document_before_the_images() {
    let env = Env::new(NodeKind::Summary);
    // A 28 MiB document leaves room for just one 1 MiB image (~1.4 MB as base64).
    let doc = "a".repeat(28 * 1024 * 1024);
    let paths: Vec<PathBuf> = (0..4u8)
        .map(|i| env.file(&format!("m{i}.jpg"), &vec![i + 1; 1024 * 1024]))
        .collect();
    let mut inputs = summary_inputs(&env, Some(paths));
    inputs.insert(
        "md".to_owned(),
        input(
            ScalarType::Md,
            false,
            vec![env.file("doc.md", doc.as_bytes())],
        ),
    );

    let mock = spawn_llm(vec![ok("요약")]).await;
    let out = summary::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs)
        .await
        .unwrap();
    assert_eq!(mock.image_blocks(0).len(), 1);
    assert!(content_length(&mock, 0) <= MAX_REQUEST_BYTES);
    assert!(
        env.output_text(&out)
            .ends_with("※ 이미지 3개는 크기/개수 제한으로 제외됨")
    );
}

#[tokio::test]
async fn summary_fails_without_a_call_when_the_text_alone_is_too_large() {
    let env = Env::new(NodeKind::Summary);
    let doc = "a".repeat(31 * 1024 * 1024);
    let inputs = NodeInputs::from([(
        "md".to_owned(),
        input(
            ScalarType::Md,
            false,
            vec![env.file("huge.md", doc.as_bytes())],
        ),
    )]);
    let mock = spawn_llm(vec![ok("never")]).await;
    let message = failed(summary::run(&client(&mock.url, Some(KEY)), &env.ctx, &inputs).await);
    assert_eq!(message, "문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다");
    assert_eq!(mock.count(), 0);
}

// ------------------------------------------------------------------ full runs through the API

struct Harness {
    app: Router,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn harness(pool: PgPool, extractor_url: &str, llm_url: &str, key: Option<&str>) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-llm-run-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let (dir, extractor_url, llm_url, key) = (
        data.to_string_lossy().into_owned(),
        extractor_url.to_owned(),
        llm_url.to_owned(),
        key.map(str::to_owned),
    );
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        "EXTRACTOR_URL" => Some(extractor_url.clone()),
        "OPENROUTER_BASE_URL" => Some(llm_url.clone()),
        "OPENROUTER_API_KEY" => key.clone(),
        _ => None,
    })
    .unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    Harness {
        app: build_app(AppState::new(pool, config, http, Arc::new(runner))),
        data,
    }
}

const EXTRACTED_MD: &str = "# Annual Report\n\nRevenue grew.";

/// Minimal extractor: writes md/json below the requested output dir.
async fn spawn_extractor() -> String {
    async fn handler(Json(body): Json<Value>) -> Response {
        let out = PathBuf::from(body["output_dir"].as_str().unwrap());
        tokio::fs::create_dir_all(&out).await.unwrap();
        tokio::fs::write(out.join("md.md"), EXTRACTED_MD)
            .await
            .unwrap();
        tokio::fs::write(out.join("json.json"), "{}").await.unwrap();
        Json(json!({"md": out.join("md.md"), "json": out.join("json.json"), "tables": [], "images": []})).into_response()
    }
    let app = Router::new().route("/extract", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
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
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
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
    let boundary = "llm-boundary";
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
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    id
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

/// bundle → pick_pdf → extract → translate(md) ← json_input(settings); translate is the output.
fn translate_graph(bundle: &str) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), false),
            node("s", "json_input", json!({"value": {"targetLanguage": "ko"}}), false),
            node("t", "translate", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "x", "pdf"),
            edge("e3", "x", "md", "t", "md"),
            edge("e4", "s", "json", "t", "settings")
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
async fn extract_then_translate_run_succeeds(pool: PgPool) {
    let extractor = spawn_extractor().await;
    let llm = spawn_llm(vec![ok("# 연간 보고서\n\n매출이 증가했습니다.")]).await;
    let h = harness(pool, &extractor, &llm.url, Some(KEY));
    let bundle = upload_bundle(&h.app).await;

    let (run, detail) = run_graph(&h.app, translate_graph(&bundle)).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");
    for id in ["b", "p", "x", "s", "t"] {
        assert_eq!(node_of(&detail, id)["status"], "succeeded", "{id}");
    }

    // The translation request carried the extracted Markdown and the settings from json_input.
    assert_eq!(llm.count(), 1);
    assert_eq!(llm.user_text(0), EXTRACTED_MD);
    assert!(
        llm.body(0)["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Target language: ko")
    );
    assert_eq!(
        llm.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::get(format!("/api/runs/{run}/artifacts/t/md"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()["content-type"],
        "text/markdown; charset=utf-8"
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        "# 연간 보고서\n\n매출이 증가했습니다."
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn missing_api_key_fails_only_the_llm_node(pool: PgPool) {
    let extractor = spawn_extractor().await;
    let llm = spawn_llm(vec![ok("unused")]).await;
    let h = harness(pool, &extractor, &llm.url, None); // engine starts fine without a key
    let bundle = upload_bundle(&h.app).await;

    let (_, detail) = run_graph(&h.app, translate_graph(&bundle)).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(node_of(&detail, "x")["status"], "succeeded");
    assert_eq!(node_of(&detail, "t")["status"], "failed");
    assert_eq!(
        node_of(&detail, "t")["error"],
        "OPENROUTER_API_KEY 가 설정되지 않았습니다"
    );
    assert_eq!(llm.count(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_key_never_leaks_through_run_results(pool: PgPool) {
    let extractor = spawn_extractor().await;
    // Upstream echoes the key in its error message.
    let llm = spawn_llm(vec![error(401, &format!("invalid key {KEY}"))]).await;
    let h = harness(pool, &extractor, &llm.url, Some(KEY));
    let bundle = upload_bundle(&h.app).await;

    let (run, detail) = run_graph(&h.app, translate_graph(&bundle)).await;
    assert_eq!(detail["status"], "failed");
    let error = node_of(&detail, "t")["error"].as_str().unwrap();
    assert!(
        error.starts_with("LLM API 오류 (HTTP 401): invalid key"),
        "{error}"
    );
    assert!(
        !detail.to_string().contains(KEY),
        "run detail leaks the key"
    );

    // Same for the run history and the SSE snapshot.
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::get(format!("/api/runs/{run}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let sse = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(!sse.contains(KEY), "SSE stream leaks the key");
}
