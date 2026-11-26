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
use dochestra_engine::clients::embedding::{EmbeddingClient, EmbeddingError};
use dochestra_engine::config::Config;
use dochestra_engine::nodes::EngineRunner;
use dochestra_engine::state::AppState;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const KEY: &str = "sk-or-secret-1";
const DIMS: usize = 1024;

// ------------------------------------------------------------------ generic mock server

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Value,
    headers: Vec<(&'static str, &'static str)>,
}

struct Recorded {
    headers: HashMap<String, String>,
    body: Value,
}

#[derive(Clone)]
struct Shared {
    /// Scripted replies consumed first; `default` answers afterwards.
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    default: fn(&Value) -> Reply,
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
}

fn new_shared(scripted: Vec<Reply>, default: fn(&Value) -> Reply) -> Shared {
    Shared {
        replies: Arc::new(Mutex::new(scripted.into())),
        requests: Arc::new(Mutex::new(Vec::new())),
        default,
    }
}

/// One server answering several endpoints (OpenRouter serves them all under one base URL); the
/// returned mocks share the URL and each records only its own endpoint's requests.
/// `(path, scripted replies, default answer)` of one endpoint.
type Route = (&'static str, Vec<Reply>, fn(&Value) -> Reply);

async fn spawn_routes(routes: Vec<Route>) -> Vec<Mock> {
    let mut app = Router::new();
    let mut shares = Vec::new();
    for (path, scripted, default) in routes {
        let shared = new_shared(scripted, default);
        app = app.route(path, post(handler).with_state(shared.clone()));
        shares.push(shared);
    }
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

async fn spawn(path: &'static str, scripted: Vec<Reply>, default: fn(&Value) -> Reply) -> Mock {
    spawn_routes(vec![(path, scripted, default)])
        .await
        .remove(0)
}

/// Embeddings and chat completions behind one base URL: `(embeddings, llm)`.
async fn spawn_providers(embedding_replies: Vec<Reply>, llm_replies: Vec<Reply>) -> (Mock, Mock) {
    let mut mocks = spawn_routes(vec![
        ("/embeddings", embedding_replies, embed_default),
        ("/chat/completions", llm_replies, llm_default),
    ])
    .await;
    let llm = mocks.remove(1);
    (mocks.remove(0), llm)
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
    let mut resp = (
        StatusCode::from_u16(reply.status).unwrap(),
        reply.body.to_string(),
    )
        .into_response();
    resp.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    for (name, value) in reply.headers {
        resp.headers_mut().insert(name, value.parse().unwrap());
    }
    resp
}

fn error_reply(status: u16, message: &str) -> Reply {
    Reply {
        status,
        body: json!({"error": {"message": message}}),
        headers: vec![],
    }
}

// ------------------------------------------------------------------ mock OpenAI embeddings

/// Axis of a text: the number in its first `@N@` marker (so tests control similarity), or a
/// fixed fallback axis.
fn axis_of(text: &str) -> usize {
    text.split('@')
        .nth(1)
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(DIMS - 1, |n| n % DIMS)
}

fn unit_vector(axis: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; DIMS];
    v[axis] = 1.0;
    v
}

/// One unit vector per input (axis from the marker), returned in REVERSED order with explicit
/// `index` fields, as the API is allowed to do.
fn embed_default(body: &Value) -> Reply {
    let inputs = body["input"].as_array().unwrap();
    let mut data: Vec<Value> = inputs
        .iter()
        .enumerate()
        .map(|(i, text)| json!({"object": "embedding", "index": i, "embedding": unit_vector(axis_of(text.as_str().unwrap()))}))
        .collect();
    data.reverse();
    Reply {
        status: 200,
        body: json!({"object": "list", "data": data}),
        headers: vec![],
    }
}

async fn spawn_embeddings(scripted: Vec<Reply>) -> Mock {
    spawn("/embeddings", scripted, embed_default).await
}

// ------------------------------------------------------------------ mock Anthropic

fn llm_default(_: &Value) -> Reply {
    Reply {
        status: 200,
        body: json!({"choices": [{"message": {"role": "assistant", "content": "ANSWER [1]"}, "finish_reason": "stop"}]}),
        headers: vec![],
    }
}

fn llm_user_text(mock: &Mock, i: usize) -> String {
    mock.body(i)["messages"][1]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["type"] == "text")
        .map(|b| b["text"].as_str().unwrap().to_owned())
        .collect()
}

// ------------------------------------------------------------------ mock extractor

#[derive(Clone)]
struct ExtractorMd(Arc<String>);

async fn spawn_extractor(md: &str) -> String {
    async fn handler(State(md): State<ExtractorMd>, Json(body): Json<Value>) -> Response {
        let out = PathBuf::from(body["output_dir"].as_str().unwrap());
        tokio::fs::create_dir_all(&out).await.unwrap();
        tokio::fs::write(out.join("md.md"), md.0.as_str())
            .await
            .unwrap();
        tokio::fs::write(out.join("json.json"), "{}").await.unwrap();
        Json(json!({"md": out.join("md.md"), "json": out.join("json.json"), "tables": [], "images": []})).into_response()
    }
    let app = Router::new()
        .route("/extract", post(handler))
        .with_state(ExtractorMd(Arc::new(md.to_owned())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
}

// ------------------------------------------------------------------ embedding client (direct)

fn embedder(url: &str, key: Option<&str>) -> EmbeddingClient {
    EmbeddingClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "baai/bge-m3",
    )
    .with_backoff_base(Duration::from_millis(5))
}

fn marked(i: usize) -> String {
    format!("text @{i}@")
}

#[tokio::test]
async fn embeddings_are_batched_by_item_count_and_keep_input_order() {
    let mock = spawn_embeddings(vec![]).await;
    let texts: Vec<String> = (0..200).map(marked).collect();
    let out = embedder(&mock.url, Some(KEY)).embed(&texts).await.unwrap();

    assert_eq!(out.len(), 200);
    for (i, vector) in out.iter().enumerate() {
        assert_eq!(vector.len(), DIMS);
        assert_eq!(vector[i], 1.0, "vector {i} must belong to input {i}");
    }
    let sizes: Vec<usize> = (0..mock.count())
        .map(|i| mock.body(i)["input"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [96, 96, 8]);
}

#[tokio::test]
async fn embeddings_are_batched_by_total_characters() {
    let mock = spawn_embeddings(vec![]).await;
    let texts: Vec<String> = (0..3)
        .map(|i| format!("@{i}@{}", "a".repeat(90_000)))
        .collect();
    let out = embedder(&mock.url, Some(KEY)).embed(&texts).await.unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(mock.count(), 2);
    assert_eq!(mock.body(0)["input"].as_array().unwrap().len(), 2);
    assert_eq!(mock.body(1)["input"].as_array().unwrap().len(), 1);

    // A single input above 200,000 characters is refused before any request.
    let mock = spawn_embeddings(vec![]).await;
    let err = embedder(&mock.url, Some(KEY))
        .embed(&["a".repeat(200_001)])
        .await
        .unwrap_err();
    assert_eq!(err, EmbeddingError::InputTooLarge { chars: 200_001 });
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn embedding_request_has_the_documented_shape() {
    let mock = spawn_embeddings(vec![]).await;
    embedder(&mock.url, Some(KEY))
        .embed(&["hello @4@".to_owned()])
        .await
        .unwrap();
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
        json!({"model": "baai/bge-m3", "input": ["hello @4@"]})
    );
}

#[tokio::test]
async fn wrong_dimensions_are_rejected() {
    let short = Reply {
        status: 200,
        body: json!({"data": [{"index": 0, "embedding": vec![0.1f32; 768]}]}),
        headers: vec![],
    };
    let mock = spawn_embeddings(vec![short]).await;
    let err = embedder(&mock.url, Some(KEY))
        .embed(&["x".to_owned()])
        .await
        .unwrap_err();
    assert_eq!(
        err,
        EmbeddingError::DimensionMismatch {
            expected: 1024,
            actual: 768
        }
    );
    assert_eq!(err.to_string(), "임베딩 차원이 768 입니다 (기대 1024)");
}

#[tokio::test]
async fn embedding_errors_are_readable_retried_when_transient_and_never_leak_the_key() {
    // 401: message propagated, not retried, key scrubbed even if the upstream echoes it.
    let mock = spawn_embeddings(vec![error_reply(
        401,
        &format!("Incorrect API key provided: {KEY}"),
    )])
    .await;
    let err = embedder(&mock.url, Some(KEY))
        .embed(&["x".to_owned()])
        .await
        .unwrap_err();
    assert!(matches!(err, EmbeddingError::Api { status: 401, .. }));
    let text = err.to_string();
    assert!(
        text.starts_with("임베딩 API 오류 (HTTP 401): Incorrect API key provided"),
        "{text}"
    );
    assert!(!text.contains(KEY) && !format!("{err:?}").contains(KEY));
    assert_eq!(mock.count(), 1);

    // 429 (retry-after: 0) then 503 then success.
    let mut limited = error_reply(429, "slow down");
    limited.headers.push(("retry-after", "0"));
    let mock = spawn_embeddings(vec![limited, error_reply(503, "busy")]).await;
    let out = embedder(&mock.url, Some(KEY))
        .embed(&["ok @2@".to_owned()])
        .await
        .unwrap();
    assert_eq!(out[0][2], 1.0);
    assert_eq!(mock.count(), 3);

    // Missing key: no HTTP call at all.
    let mock = spawn_embeddings(vec![]).await;
    let err = embedder(&mock.url, None)
        .embed(&["x".to_owned()])
        .await
        .unwrap_err();
    assert_eq!(err, EmbeddingError::MissingApiKey);
    assert_eq!(err.to_string(), "OPENROUTER_API_KEY 가 설정되지 않았습니다");
    assert_eq!(mock.count(), 0);

    // Nothing to embed: no call and no key needed.
    assert!(
        embedder(&mock.url, None)
            .embed(&[])
            .await
            .unwrap()
            .is_empty()
    );
}

// ------------------------------------------------------------------ API-level tests

struct Harness {
    app: Router,
    pool: PgPool,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

struct Urls<'a> {
    extractor: &'a str,
    /// OpenRouter base URL (embeddings and chat completions share it).
    base: &'a str,
    key: Option<&'a str>,
}

fn harness(pool: PgPool, urls: &Urls<'_>) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-rag-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let env: HashMap<&str, Option<String>> = HashMap::from([
        ("DATABASE_URL", Some("unused".to_owned())),
        ("DATA_DIR", Some(data.to_string_lossy().into_owned())),
        ("EXTRACTOR_URL", Some(urls.extractor.to_owned())),
        ("OPENROUTER_BASE_URL", Some(urls.base.to_owned())),
        ("OPENROUTER_API_KEY", urls.key.map(str::to_owned)),
    ]);
    let config = Config::from_lookup(move |k| env.get(k).cloned().flatten()).unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    let app = build_app(AppState::new(pool.clone(), config, http, Arc::new(runner)));
    Harness { app, pool, data }
}

async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    (
        status,
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
    let (status, bytes) = raw(app, builder.body(body).unwrap()).await;
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
    let boundary = "rag-boundary";
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
    let (status, _) = raw(app, req).await;
    assert_eq!(status, StatusCode::CREATED);
    id
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

/// bundle → pick_pdf → extract → rag (output)
fn rag_graph(bundle: &str) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), false),
            node("r", "rag", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "x", "pdf"),
            edge("e3", "x", "md", "r", "md")
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

/// Three ~1000-character paragraphs (too big to share a 1500-char chunk), each marked with the
/// axis its mock embedding will use.
fn three_paragraph_doc() -> (String, Vec<String>) {
    let paragraphs: Vec<String> = ["alpha", "bravo", "charlie"]
        .iter()
        .enumerate()
        .map(|(i, word)| format!("{word} @{i}@ {}", "lorem ".repeat(160)))
        .collect();
    (paragraphs.join("\n\n"), paragraphs)
}

struct Indexed {
    h: Harness,
    index_id: String,
    run_id: String,
    embeddings: Mock,
    llm: Mock,
    paragraphs: Vec<String>,
}

/// Runs bundle → pick → extract → rag against the mocks and returns the created index.
async fn indexed_document(pool: PgPool) -> Indexed {
    let (doc, paragraphs) = three_paragraph_doc();
    let extractor = spawn_extractor(&doc).await;
    let (embeddings, llm) = spawn_providers(vec![], vec![]).await;
    let h = harness(
        pool,
        &Urls {
            extractor: &extractor,
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let bundle = upload_bundle(&h.app).await;
    let (run_id, detail) = run_graph(&h.app, rag_graph(&bundle)).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");
    let (status, bytes) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run_id}/artifacts/r/api"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let api: Value = serde_json::from_slice(&bytes).unwrap();
    let index_id = api["indexId"].as_str().unwrap().to_owned();
    Indexed {
        h,
        index_id,
        run_id,
        embeddings,
        llm,
        paragraphs,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn rag_node_stores_chunks_and_describes_the_query_api(pool: PgPool) {
    let ix = indexed_document(pool).await;

    // The node's output follows CONTEXT §4.3.
    let (status, bytes) = raw(
        &ix.h.app,
        Request::get(format!("/api/runs/{}/artifacts/r/api", ix.run_id))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let api: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        api,
        json!({
            "type": "rag_api",
            "indexId": ix.index_id,
            "method": "POST",
            "endpoint": format!("/api/rag/{}/query", ix.index_id),
            "request": {"question": "string", "topK": "number (선택, 기본 5, 1~20)"},
            "chunkCount": 3,
            "embeddingModel": "baai/bge-m3"
        })
    );

    // The DB holds the index and exactly chunkCount chunks, in order, with 1024-dim vectors.
    let index: (Uuid, String, String) = sqlx::query_as(
        "SELECT run_id, node_id, embedding_model FROM rag_indexes WHERE id = $1::uuid",
    )
    .bind(&ix.index_id)
    .fetch_one(&ix.h.pool)
    .await
    .unwrap();
    assert_eq!(
        (index.0.to_string(), index.1.as_str(), index.2.as_str()),
        (ix.run_id.clone(), "r", "baai/bge-m3")
    );
    let rows: Vec<(i32, String, i32)> = sqlx::query_as(
        "SELECT chunk_index, content, vector_dims(embedding) FROM rag_chunks WHERE index_id = $1::uuid ORDER BY chunk_index",
    )
    .bind(&ix.index_id)
    .fetch_all(&ix.h.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    for (i, (chunk_index, content, dims)) in rows.iter().enumerate() {
        assert_eq!(*chunk_index as usize, i);
        assert_eq!(content, &ix.paragraphs[i]);
        assert_eq!(*dims, 1024);
    }

    // One embeddings request carried all three chunks, authenticated with the configured key.
    assert_eq!(ix.embeddings.count(), 1);
    assert_eq!(ix.embeddings.body(0)["input"].as_array().unwrap().len(), 3);
    assert_eq!(
        ix.embeddings.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn query_returns_the_closest_chunk_first_and_answers_from_it(pool: PgPool) {
    let ix = indexed_document(pool).await;
    let uri = format!("/api/rag/{}/query", ix.index_id);

    let (status, body) = call(
        &ix.h.app,
        "POST",
        &uri,
        Some(json!({"question": "Tell me about bravo @1@", "topK": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["answer"], "ANSWER [1]");
    let sources = body["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["chunkIndex"], 1);
    assert_eq!(sources[0]["content"], ix.paragraphs[1].as_str());
    assert!(
        (sources[0]["score"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{}",
        sources[0]["score"]
    );
    assert!(sources[1]["score"].as_f64().unwrap().abs() < 1e-6); // orthogonal chunk
    assert_eq!(sources[1]["chunkIndex"], 0); // ties break by chunk index
    assert!(sources[0]["score"].as_f64() >= sources[1]["score"].as_f64());

    // The prompt sent to Claude holds the numbered top chunks and the question.
    assert_eq!(ix.llm.count(), 1);
    let prompt = llm_user_text(&ix.llm, 0);
    assert!(
        prompt.contains(&format!("[1]\n{}", ix.paragraphs[1])),
        "{prompt}"
    );
    assert!(prompt.contains(&format!("[2]\n{}", ix.paragraphs[0])));
    assert!(
        !prompt.contains(&ix.paragraphs[2]),
        "only topK chunks are passed"
    );
    assert!(prompt.contains("Question: Tell me about bravo @1@"));
    let system = ix.llm.body(0)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        system.contains("only the information in the context")
            || system.contains("only the numbered context")
    );
    assert_eq!(
        ix.llm.header(0, "authorization").as_deref(),
        Some(&*format!("Bearer {KEY}"))
    );

    // Default topK is 5 (all 3 chunks here); the best match changes with the question.
    let (_, body) = call(
        &ix.h.app,
        "POST",
        &uri,
        Some(json!({"question": "charlie? @2@"})),
    )
    .await;
    let sources = body["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 3);
    assert_eq!(sources[0]["chunkIndex"], 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn query_validation_and_missing_index(pool: PgPool) {
    let ix = indexed_document(pool).await;
    let uri = format!("/api/rag/{}/query", ix.index_id);
    let llm_calls_before = ix.llm.count();
    let embed_calls_before = ix.embeddings.count();

    for body in [
        json!({"question": ""}),
        json!({"question": "   "}),
        json!({"question": "q".repeat(2001)}),
        json!({"question": "q", "topK": 0}),
        json!({"question": "q", "topK": 21}),
        json!({"question": "q", "topK": -1}),
        json!({"topK": 3}),
    ] {
        let (status, err) = call(&ix.h.app, "POST", &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(err["error"]["code"], "BAD_REQUEST");
    }
    // Invalid requests never reach the providers.
    assert_eq!(ix.llm.count(), llm_calls_before);
    assert_eq!(ix.embeddings.count(), embed_calls_before);

    // Boundaries are accepted.
    for body in [
        json!({"question": "q @0@", "topK": 1}),
        json!({"question": "q @0@", "topK": 20}),
        json!({"question": "가".repeat(2000)}),
    ] {
        let (status, _) = call(&ix.h.app, "POST", &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, err) = call(
        &ix.h.app,
        "POST",
        &format!("/api/rag/{}/query", Uuid::new_v4()),
        Some(json!({"question": "q"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(err["error"]["code"], "NOT_FOUND");
    let (status, err) = call(
        &ix.h.app,
        "POST",
        "/api/rag/not-a-uuid/query",
        Some(json!({"question": "q"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["code"], "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn query_maps_provider_failures_to_502_without_leaking_the_key(pool: PgPool) {
    let ix = indexed_document(pool.clone()).await;
    let uri = format!("/api/rag/{}/query", ix.index_id);

    // No key configured at query time (the index itself was built with one): nothing is called.
    let (embeddings, llm) = spawn_providers(vec![], vec![]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: "http://127.0.0.1:9",
            base: &embeddings.url,
            key: None,
        },
    );
    let (status, err) = call(&h.app, "POST", &uri, Some(json!({"question": "q"}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(err["error"]["code"], "UPSTREAM");
    assert_eq!(
        err["error"]["message"],
        "OPENROUTER_API_KEY 가 설정되지 않았습니다"
    );
    assert_eq!((embeddings.count(), llm.count()), (0, 0));

    // An embedding error that echoes the key is scrubbed.
    let (embeddings, _llm) =
        spawn_providers(vec![error_reply(401, &format!("bad key {KEY}"))], vec![]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: "http://127.0.0.1:9",
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let (status, err) = call(&h.app, "POST", &uri, Some(json!({"question": "q"}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let message = err["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("임베딩 API 오류 (HTTP 401): bad key"),
        "{message}"
    );
    assert!(!err.to_string().contains(KEY));

    // Same for the answering step; retrieval worked, the LLM call fails.
    let (embeddings, llm) =
        spawn_providers(vec![], vec![error_reply(400, &format!("invalid {KEY}"))]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: "http://127.0.0.1:9",
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let (status, err) = call(&h.app, "POST", &uri, Some(json!({"question": "q"}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("LLM API 오류 (HTTP 400): invalid")
    );
    assert!(!err.to_string().contains(KEY));
    assert_eq!((embeddings.count(), llm.count()), (1, 1));

    // An empty answer is a failure too, not an empty 200.
    let empty = Reply {
        status: 200,
        body: json!({"choices": [{"message": {"content": ""}, "finish_reason": "stop"}]}),
        headers: vec![],
    };
    let (embeddings, _llm) = spawn_providers(vec![], vec![empty]).await;
    let h = harness(
        pool,
        &Urls {
            extractor: "http://127.0.0.1:9",
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let (status, err) = call(&h.app, "POST", &uri, Some(json!({"question": "q"}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(err["error"]["message"], "LLM 응답이 비어 있습니다");
}

#[sqlx::test(migrations = "./migrations")]
async fn rag_node_fails_for_empty_documents_and_missing_embedding_key(pool: PgPool) {
    // Empty / whitespace-only Markdown.
    let extractor = spawn_extractor("  \n\n \n").await;
    let (embeddings, _llm) = spawn_providers(vec![], vec![]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: &extractor,
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let bundle = upload_bundle(&h.app).await;
    let (_, detail) = run_graph(&h.app, rag_graph(&bundle)).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(node_of(&detail, "x")["status"], "succeeded");
    assert_eq!(node_of(&detail, "r")["error"], "문서 내용이 비어 있습니다");
    assert_eq!(embeddings.count(), 0);
    let indexes: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_indexes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(indexes, 0);

    // No embedding key.
    let (doc, _) = three_paragraph_doc();
    let extractor = spawn_extractor(&doc).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: &extractor,
            base: &embeddings.url,
            key: None,
        },
    );
    let bundle = upload_bundle(&h.app).await;
    let (_, detail) = run_graph(&h.app, rag_graph(&bundle)).await;
    assert_eq!(node_of(&detail, "r")["status"], "failed");
    assert_eq!(
        node_of(&detail, "r")["error"],
        "OPENROUTER_API_KEY 가 설정되지 않았습니다"
    );
    let indexes: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_indexes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(indexes, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_embedding_stores_nothing_and_reports_the_api_message(pool: PgPool) {
    let (doc, _) = three_paragraph_doc();
    let extractor = spawn_extractor(&doc).await;
    let (embeddings, _llm) =
        spawn_providers(vec![error_reply(400, "input is invalid")], vec![]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: &extractor,
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let bundle = upload_bundle(&h.app).await;
    let (_, detail) = run_graph(&h.app, rag_graph(&bundle)).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(
        node_of(&detail, "r")["error"],
        "임베딩 API 오류 (HTTP 400): input is invalid"
    );
    let chunks: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_chunks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(chunks, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn search_is_scoped_to_one_index(pool: PgPool) {
    // Two documents indexed in the same table; each query must only see its own chunks.
    let first = indexed_document(pool.clone()).await;
    let second = indexed_document(pool).await;
    assert_ne!(first.index_id, second.index_id);
    let (_, body) = call(
        &second.h.app,
        "POST",
        &format!("/api/rag/{}/query", second.index_id),
        Some(json!({"question": "x @1@", "topK": 20})),
    )
    .await;
    assert_eq!(body["sources"].as_array().unwrap().len(), 3);
    let (_, body) = call(
        &first.h.app,
        "POST",
        &format!("/api/rag/{}/query", first.index_id),
        Some(json!({"question": "x @1@", "topK": 20})),
    )
    .await;
    assert_eq!(body["sources"].as_array().unwrap().len(), 3);
}

/// Regression for the live-check findings: long paragraphs were hard-cut at 1500 characters
/// (mid-sentence, even mid-word) leaving tiny headless tails.
#[sqlx::test(migrations = "./migrations")]
async fn long_sections_are_stored_as_balanced_sentence_aligned_chunks_with_their_heading(
    pool: PgPool,
) {
    let sentences: String = (0..70)
        .map(|i| format!("Sentence {i:02} explains how the fleet controller @{i}@ plans routes. "))
        .collect();
    let md = format!(
        "# Handbook\n\n## 8.4 Upgrades and Support\n\n{}\n\n## 9. Appendix\n\nShort closing remark.",
        sentences.trim()
    );
    let extractor = spawn_extractor(&md).await;
    let (embeddings, _llm) = spawn_providers(vec![], vec![]).await;
    let h = harness(
        pool.clone(),
        &Urls {
            extractor: &extractor,
            base: &embeddings.url,
            key: Some(KEY),
        },
    );
    let bundle = upload_bundle(&h.app).await;
    let (run_id, detail) = run_graph(&h.app, rag_graph(&bundle)).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");

    let (_, bytes) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run_id}/artifacts/r/api"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let api: Value = serde_json::from_slice(&bytes).unwrap();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT content FROM rag_chunks WHERE index_id = $1::uuid ORDER BY chunk_index",
    )
    .bind(api["indexId"].as_str().unwrap())
    .fetch_all(&pool)
    .await
    .unwrap();
    let chunks: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(chunks.len() as u64, api["chunkCount"].as_u64().unwrap());

    let section: Vec<&&str> = chunks
        .iter()
        .filter(|c| c.contains("explains how the fleet controller"))
        .collect();
    assert!(section.len() >= 3, "{} chunks", section.len());
    for chunk in &section {
        assert!(chunk.chars().count() <= 1500, "{}", chunk.chars().count());
        let body = chunk.strip_prefix("# Handbook\n\n").unwrap_or(chunk);
        assert!(
            body.starts_with("## 8.4 Upgrades and Support\n\nSentence "),
            "starts at a sentence under its heading: {:?}",
            &chunk[..60]
        );
        assert!(
            chunk.trim_end().ends_with("plans routes."),
            "ends at a sentence: {:?}",
            &chunk[chunk.len() - 40..]
        );
    }
    let sizes: Vec<usize> = section.iter().map(|c| c.chars().count()).collect();
    assert!(
        sizes.iter().max().unwrap() - sizes.iter().min().unwrap() < 250,
        "uneven pieces: {sizes:?}"
    );
    // Every sentence is stored exactly once.
    for i in 0..70 {
        let marker = format!("Sentence {i:02} ");
        assert_eq!(
            chunks.iter().filter(|c| c.contains(&marker)).count(),
            1,
            "{marker}"
        );
    }
    // The next section starts its own chunk with its own heading (no stray context).
    assert!(chunks.last().unwrap().starts_with("## 9. Appendix"));
}
