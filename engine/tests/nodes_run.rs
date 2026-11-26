use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use dochestra_engine::build_app;
use dochestra_engine::clients::extractor::ExtractorClient;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs,
};
use dochestra_engine::nodes::{EngineRunner, extract};
use dochestra_engine::state::AppState;
use dochestra_engine::types::{DataType, NodeKind, ScalarType};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

const BOUNDARY: &str = "dochestra-test-boundary";

struct Harness {
    app: Router,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn harness(pool: PgPool) -> Harness {
    harness_with(pool, None)
}

fn harness_with(pool: PgPool, extractor_url: Option<String>) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-nodes-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let dir = data.to_string_lossy().into_owned();
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        "EXTRACTOR_URL" => extractor_url.clone(),
        _ => None,
    })
    .unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    let app = build_app(AppState::new(pool, config, http, Arc::new(runner)));
    Harness { app, data }
}

async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
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

fn pdf_bytes(tag: &str) -> Vec<u8> {
    format!("%PDF-1.7\n{tag}\n%%EOF\n").into_bytes()
}

/// Creates a bundle and uploads the given PDFs through the real API.
async fn bundle_with(app: &Router, pdfs: &[(&str, Vec<u8>)]) -> String {
    let (_, bundle) = call(app, "POST", "/api/bundles", Some(json!({"name": "b"}))).await;
    let id = bundle["id"].as_str().unwrap().to_owned();
    let mut body = Vec::new();
    for (name, content) in pdfs {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    let req = Request::post(format!("/api/bundles/{id}/files"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, _, resp) = raw(app, req).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&resp)
    );
    id
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

/// Saves the graph, runs it and waits for the run to finish; returns (run id, detail).
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
async fn bundle_then_pick_pdf_returns_the_uploaded_bytes(pool: PgPool) {
    let h = harness(pool);
    let (first, second) = (pdf_bytes("first"), pdf_bytes("second"));
    let bundle = bundle_with(
        &h.app,
        &[("first.pdf", first.clone()), ("second.pdf", second.clone())],
    )
    .await;

    for (index, expected) in [(0, &first), (1, &second)] {
        let graph = json!({
            "nodes": [
                node("b", "bundle", json!({"bundleId": bundle}), false),
                node("p", "pick_pdf", json!({"index": index}), true)
            ],
            "edges": [edge("e1", "b", "files", "p", "files")]
        });
        let (run, detail) = run_graph(&h.app, graph).await;
        assert_eq!(detail["status"], "succeeded", "{detail}");
        assert_eq!(node_of(&detail, "b")["status"], "succeeded");
        assert_eq!(
            node_of(&detail, "b")["outputs"][0]["files"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let (status, headers, body) = raw(
            &h.app,
            Request::get(format!("/api/runs/{run}/artifacts/p/pdf"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "application/pdf");
        assert_eq!(&body, expected, "index {index}");
    }

    // The bundle node's list output is downloadable too (served straight from bundles/).
    let graph = json!({
        "nodes": [node("b", "bundle", json!({"bundleId": bundle}), true)],
        "edges": []
    });
    let (run, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "succeeded");
    let (status, _, body) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run}/artifacts/b/files?index=1"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, second);
}

#[sqlx::test(migrations = "./migrations")]
async fn json_input_output_is_downloadable_json(pool: PgPool) {
    let h = harness(pool);
    let value = json!({"targetLanguage": "ko", "glossary": {"run": "실행"}});
    let graph =
        json!({"nodes": [node("j", "json_input", json!({"value": value}), true)], "edges": []});
    let (run, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");

    let (status, headers, body) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run}/artifacts/j/json"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), value);
}

#[sqlx::test(migrations = "./migrations")]
async fn pick_out_of_range_fails_the_node(pool: PgPool) {
    let h = harness(pool);
    let bundle = bundle_with(&h.app, &[("only.pdf", pdf_bytes("only"))]).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 5}), true)
        ],
        "edges": [edge("e1", "b", "files", "p", "files")]
    });
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(node_of(&detail, "b")["status"], "succeeded");
    assert_eq!(node_of(&detail, "p")["status"], "failed");
    assert_eq!(
        node_of(&detail, "p")["error"],
        "index 5 가 범위를 벗어났습니다 (항목 1개)"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unimplemented_kinds_still_fail_while_upstream_succeeds(pool: PgPool) {
    let h = harness(pool);
    let bundle = bundle_with(&h.app, &[("a.pdf", pdf_bytes("a"))]).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("j", "json_input", json!({"value": {"name": "x"}}), false),
            node("f", "fill", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "f", "pdf"),
            edge("e3", "j", "json", "f", "values")
        ]
    });
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "failed");
    for id in ["b", "p", "j"] {
        assert_eq!(node_of(&detail, id)["status"], "succeeded", "{id}");
    }
    assert_eq!(node_of(&detail, "f")["status"], "failed");
    assert_eq!(
        node_of(&detail, "f")["error"],
        "노드 'fill' 는 아직 구현되지 않았습니다"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn bundle_node_reports_missing_bundle_and_empty_bundle(pool: PgPool) {
    let h = harness(pool);
    let ghost = Uuid::new_v4();
    let graph =
        json!({"nodes": [node("b", "bundle", json!({"bundleId": ghost}), true)], "edges": []});
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(
        node_of(&detail, "b")["error"],
        format!("번들 {ghost} 를 찾을 수 없습니다")
    );

    let (_, empty) = call(
        &h.app,
        "POST",
        "/api/bundles",
        Some(json!({"name": "empty"})),
    )
    .await;
    let graph = json!({"nodes": [node("b", "bundle", json!({"bundleId": empty["id"]}), true)], "edges": []});
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(node_of(&detail, "b")["error"], "번들에 PDF 가 없습니다");
}

// ------------------------------------------------------------------ extract node (mock extractor)

#[derive(Clone)]
enum Behavior {
    /// 200 with real files written below the requested `output_dir`.
    Files { tables: usize, images: usize },
    /// Error status with a raw body.
    Status(u16, String),
    /// Like `Files` but answers after a delay.
    Delay(u64),
    /// 200, but `md` points at this path instead.
    Redirect(PathBuf),
    /// 200 with a body that is not an ExtractResponse.
    Garbage,
}

struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

type MockState = (Behavior, Arc<Mutex<Vec<Value>>>);

async fn spawn_mock(behavior: Behavior) -> Mock {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/extract", post(mock_extract))
        .with_state((behavior, requests.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock { url, requests }
}

async fn mock_extract(
    State((behavior, requests)): State<MockState>,
    Json(body): Json<Value>,
) -> Response {
    requests.lock().unwrap().push(body.clone());
    let out = PathBuf::from(body["output_dir"].as_str().unwrap());
    let (tables, images, md_override) = match &behavior {
        Behavior::Files { tables, images } => (*tables, *images, None),
        Behavior::Delay(ms) => {
            tokio::time::sleep(Duration::from_millis(*ms)).await;
            (1, 1, None)
        }
        Behavior::Redirect(path) => (0, 0, Some(path.clone())),
        Behavior::Status(code, text) => {
            return (StatusCode::from_u16(*code).unwrap(), text.clone()).into_response();
        }
        Behavior::Garbage => return Json(json!({"unexpected": true})).into_response(),
    };

    tokio::fs::create_dir_all(out.join("tables")).await.unwrap();
    tokio::fs::create_dir_all(out.join("images")).await.unwrap();
    let md = out.join("md.md");
    let json_path = out.join("json.json");
    tokio::fs::write(&md, "# extracted").await.unwrap();
    tokio::fs::write(&json_path, "{}").await.unwrap();
    let mut table_paths = Vec::new();
    for i in 0..tables {
        let p = out.join(format!("tables/{i:04}.csv"));
        tokio::fs::write(&p, "a,b\n1,2\n").await.unwrap();
        table_paths.push(p);
    }
    let mut image_paths = Vec::new();
    for i in 0..images {
        let p = out.join(format!("images/{i:04}.jpg"));
        tokio::fs::write(&p, b"\xff\xd8\xff").await.unwrap();
        image_paths.push(p);
    }
    Json(json!({
        "md": md_override.unwrap_or(md),
        "json": json_path,
        "tables": table_paths,
        "images": image_paths,
    }))
    .into_response()
}

/// Data dir + node dir + a PDF input, for calling `extract::run` directly.
struct NodeEnv {
    data_dir: PathBuf,
    ctx: NodeContext,
    inputs: NodeInputs,
    pdf: PathBuf,
}

impl NodeEnv {
    fn new() -> Self {
        let data_dir = std::env::temp_dir().join(format!("dochestra-extract-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let out_dir = data_dir.join("runs").join(run_id.to_string()).join("x");
        std::fs::create_dir_all(&out_dir).unwrap();
        let pdf = data_dir.join("bundles").join("b").join("f.pdf");
        std::fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        std::fs::write(&pdf, pdf_bytes("input")).unwrap();
        let inputs = NodeInputs::from([(
            "pdf".to_owned(),
            Artifact {
                data_type: DataType::single(ScalarType::Pdf),
                paths: vec![pdf.clone()],
            },
        )]);
        let ctx = NodeContext {
            run_id,
            node_id: "x".to_owned(),
            kind: NodeKind::Extract,
            params: json!({}),
            out_dir,
        };
        Self {
            data_dir,
            ctx,
            inputs,
            pdf,
        }
    }

    async fn run(&self, url: &str, timeout: Duration) -> Result<NodeOutputs, NodeError> {
        let client = ExtractorClient::new(reqwest::Client::new(), url, timeout);
        extract::run(&client, &self.data_dir, &self.ctx, &self.inputs).await
    }
}

impl Drop for NodeEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

const LONG: Duration = Duration::from_secs(30);

fn failed(result: Result<NodeOutputs, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(message)) => message,
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn extract_builds_the_four_output_sockets() {
    let env = NodeEnv::new();
    let mock = spawn_mock(Behavior::Files {
        tables: 2,
        images: 3,
    })
    .await;
    let out = env.run(&mock.url, LONG).await.unwrap();
    let dir = &env.ctx.out_dir;

    assert_eq!(out.len(), 4);
    assert_eq!(out["md"].data_type, DataType::single(ScalarType::Md));
    assert_eq!(out["md"].paths, vec![dir.join("md.md")]);
    assert_eq!(out["json"].data_type, DataType::single(ScalarType::Json));
    assert_eq!(out["json"].paths, vec![dir.join("json.json")]);
    assert_eq!(out["tables"].data_type, DataType::list_of(ScalarType::Csv));
    assert_eq!(
        out["tables"].paths,
        vec![dir.join("tables/0000.csv"), dir.join("tables/0001.csv")]
    );
    assert_eq!(out["images"].data_type, DataType::list_of(ScalarType::Jpg));
    assert_eq!(out["images"].paths.len(), 3);
    assert!(out["images"].paths[2].ends_with("images/0002.jpg"));

    // The request carries the PDF path and this node's output directory.
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0],
        json!({"pdf_path": env.pdf.to_str().unwrap(), "output_dir": dir.to_str().unwrap()})
    );
}

#[tokio::test]
async fn extract_without_tables_or_images_yields_empty_lists() {
    let env = NodeEnv::new();
    let mock = spawn_mock(Behavior::Files {
        tables: 0,
        images: 0,
    })
    .await;
    let out = env.run(&mock.url, LONG).await.unwrap();
    assert!(out["tables"].paths.is_empty());
    assert!(out["images"].paths.is_empty());
    assert_eq!(out["md"].paths.len(), 1);
}

#[tokio::test]
async fn extractor_http_errors_become_readable_failures() {
    let env = NodeEnv::new();
    let cases = [
        (
            500,
            r#"{"detail":"RuntimeError: boom"}"#,
            "extractor 오류 (HTTP 500): RuntimeError: boom",
        ),
        (
            400,
            r#"{"detail":"pdf not found: /data/x.pdf"}"#,
            "extractor 오류 (HTTP 400): pdf not found: /data/x.pdf",
        ),
        (
            503,
            r#"{"detail":"converter is still loading"}"#,
            "extractor 오류 (HTTP 503): converter is still loading",
        ),
        (
            422,
            r#"{"detail":[{"msg":"Field required"}]}"#,
            r#"extractor 오류 (HTTP 422): [{"msg":"Field required"}]"#,
        ),
        (502, "Bad Gateway", "extractor 오류 (HTTP 502): Bad Gateway"),
    ];
    for (status, body, expected) in cases {
        let mock = spawn_mock(Behavior::Status(status, body.to_owned())).await;
        assert_eq!(failed(env.run(&mock.url, LONG).await), expected);
    }
}

#[tokio::test]
async fn slow_extractor_times_out() {
    let env = NodeEnv::new();
    let mock = spawn_mock(Behavior::Delay(3000)).await;
    let message = failed(env.run(&mock.url, Duration::from_secs(1)).await);
    assert_eq!(message, "PDF 추출 시간 초과 (1초)");
}

#[tokio::test]
async fn unreachable_extractor_is_a_connect_failure() {
    let env = NodeEnv::new();
    let message = failed(env.run("http://127.0.0.1:9", LONG).await);
    assert!(
        message.starts_with("extractor 에 연결할 수 없습니다: "),
        "{message}"
    );
}

#[tokio::test]
async fn undecodable_response_fails() {
    let env = NodeEnv::new();
    let mock = spawn_mock(Behavior::Garbage).await;
    let message = failed(env.run(&mock.url, LONG).await);
    assert!(
        message.starts_with("extractor 응답을 해석할 수 없습니다: "),
        "{message}"
    );
}

#[tokio::test]
async fn paths_outside_the_node_dir_are_rejected() {
    let env = NodeEnv::new();

    // Outside DATA_DIR entirely.
    let outside = env
        .data_dir
        .parent()
        .unwrap()
        .join(format!("outside-{}.md", Uuid::new_v4()));
    std::fs::write(&outside, "secret").unwrap();
    let mock = spawn_mock(Behavior::Redirect(outside.clone())).await;
    let message = failed(env.run(&mock.url, LONG).await);
    assert!(
        message.starts_with("extractor 가 예상 밖 경로를 반환했습니다: "),
        "{message}"
    );
    assert!(message.contains(outside.to_str().unwrap()));
    std::fs::remove_file(&outside).unwrap();

    // Inside DATA_DIR but belonging to another node.
    let other = env.data_dir.join("runs").join("other-run").join("n");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("md.md"), "not yours").unwrap();
    let mock = spawn_mock(Behavior::Redirect(other.join("md.md"))).await;
    let message = failed(env.run(&mock.url, LONG).await);
    assert!(
        message.starts_with("extractor 가 예상 밖 경로를 반환했습니다: "),
        "{message}"
    );

    // `..` traversal that resolves outside the node dir.
    let sneaky = env.ctx.out_dir.join("../../other-run/n/md.md");
    let mock = spawn_mock(Behavior::Redirect(sneaky)).await;
    let message = failed(env.run(&mock.url, LONG).await);
    assert!(
        message.starts_with("extractor 가 예상 밖 경로를 반환했습니다: "),
        "{message}"
    );

    // A returned file that does not exist.
    let ghost = env.ctx.out_dir.join("ghost.md");
    let mock = spawn_mock(Behavior::Redirect(ghost)).await;
    let message = failed(env.run(&mock.url, LONG).await);
    assert!(
        message.starts_with("extractor 가 반환한 파일이 없습니다: "),
        "{message}"
    );
}

#[tokio::test]
async fn missing_pdf_input_fails_without_calling_the_extractor() {
    let mut env = NodeEnv::new();
    env.inputs.clear();
    let mock = spawn_mock(Behavior::Files {
        tables: 0,
        images: 0,
    })
    .await;
    assert_eq!(
        failed(env.run(&mock.url, LONG).await),
        "입력 pdf 이(가) 없습니다"
    );
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn bundle_pick_extract_run_succeeds_with_mock_extractor(pool: PgPool) {
    let mock = spawn_mock(Behavior::Files {
        tables: 1,
        images: 2,
    })
    .await;
    let h = harness_with(pool, Some(mock.url.clone()));
    let bundle = bundle_with(&h.app, &[("doc.pdf", pdf_bytes("doc"))]).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), true)
        ],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    });
    let (run, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");

    let outputs = node_of(&detail, "x")["outputs"].as_array().unwrap();
    let sockets: Vec<&str> = outputs
        .iter()
        .map(|o| o["socketId"].as_str().unwrap())
        .collect();
    assert_eq!(sockets, ["md", "json", "tables", "images"]);
    assert_eq!(outputs[3]["files"].as_array().unwrap().len(), 2);

    let (status, headers, body) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run}/artifacts/x/md"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/markdown; charset=utf-8");
    assert_eq!(body, b"# extracted");

    // The extractor was asked for the bundle's stored PDF and this node's output directory.
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let pdf_path = requests[0]["pdf_path"].as_str().unwrap();
    assert!(
        pdf_path.starts_with(h.data.to_str().unwrap()) && pdf_path.ends_with(".pdf"),
        "{pdf_path}"
    );
    assert_eq!(
        requests[0]["output_dir"].as_str().unwrap(),
        h.data.join("runs").join(&run).join("x").to_str().unwrap()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn extractor_failure_fails_the_run_with_its_detail(pool: PgPool) {
    let mock = spawn_mock(Behavior::Status(
        500,
        r#"{"detail":"RuntimeError: boom"}"#.to_owned(),
    ))
    .await;
    let h = harness_with(pool, Some(mock.url.clone()));
    let bundle = bundle_with(&h.app, &[("doc.pdf", pdf_bytes("doc"))]).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), true)
        ],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    });
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(
        node_of(&detail, "x")["error"],
        "extractor 오류 (HTTP 500): RuntimeError: boom"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn run_accepts_extract_output_with_empty_lists(pool: PgPool) {
    let mock = spawn_mock(Behavior::Files {
        tables: 0,
        images: 0,
    })
    .await;
    let h = harness_with(pool, Some(mock.url.clone()));
    let bundle = bundle_with(&h.app, &[("doc.pdf", pdf_bytes("doc"))]).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), true)
        ],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    });
    let (_, detail) = run_graph(&h.app, graph).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");
    let outputs = node_of(&detail, "x")["outputs"].as_array().unwrap();
    assert_eq!(outputs[2]["socketId"], "tables");
    assert_eq!(outputs[2]["files"], json!([]));
    assert_eq!(outputs[3]["files"], json!([]));
}
