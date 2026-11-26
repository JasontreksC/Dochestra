use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::catalog::spec;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs, NodeRunner,
};
use dochestra_engine::state::AppState;
use dochestra_engine::storage::{list_output_path, single_output_path};
use futures::FutureExt;
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const BUNDLE_ID: &str = "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f";

/// Writes a file per catalog output socket whose content is `"{node}:{socket}"`.
struct FileRunner;

impl NodeRunner for FileRunner {
    fn run(
        &self,
        ctx: NodeContext,
        _inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
        async move {
            let mut outputs = NodeOutputs::new();
            for socket in &spec(ctx.kind).outputs {
                let scalar = socket.data_type.scalar;
                let path = if socket.data_type.list {
                    let p = list_output_path(&ctx.out_dir, &socket.id, scalar, 0);
                    tokio::fs::create_dir_all(p.parent().unwrap())
                        .await
                        .unwrap();
                    p
                } else {
                    single_output_path(&ctx.out_dir, &socket.id, scalar)
                };
                tokio::fs::write(&path, format!("{}:{}", ctx.node_id, socket.id))
                    .await
                    .unwrap();
                outputs.insert(
                    socket.id.clone(),
                    Artifact {
                        data_type: socket.data_type,
                        paths: vec![path],
                    },
                );
            }
            Ok(outputs)
        }
        .boxed()
    }
}

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

fn harness(pool: PgPool) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-artifacts-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let dir = data.to_string_lossy().into_owned();
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        _ => None,
    })
    .unwrap();
    let app = build_app(AppState::new(
        pool.clone(),
        config,
        reqwest::Client::new(),
        Arc::new(FileRunner),
    ));
    Harness { app, pool, data }
}

async fn json_call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
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

struct Download {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn download(app: &Router, uri: &str) -> Download {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Download {
        status,
        headers,
        body,
    }
}

fn error_code(d: &Download) -> String {
    let v: Value = serde_json::from_slice(&d.body).unwrap();
    v["error"]["code"].as_str().unwrap().to_owned()
}

async fn completed_run(h: &Harness) -> String {
    let graph = json!({
        "nodes": [
            {"id": "b", "kind": "bundle", "position": {"x": 0, "y": 0}, "params": {"bundleId": BUNDLE_ID}, "isOutput": false},
            {"id": "p", "kind": "pick_pdf", "position": {"x": 0, "y": 0}, "params": {"index": 0}, "isOutput": false},
            {"id": "x", "kind": "extract", "position": {"x": 0, "y": 0}, "params": {}, "isOutput": true}
        ],
        "edges": [
            {"id": "e1", "source": "b", "sourceHandle": "files", "target": "p", "targetHandle": "files"},
            {"id": "e2", "source": "p", "sourceHandle": "pdf", "target": "x", "targetHandle": "pdf"}
        ]
    });
    let (_, wf) = json_call(&h.app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let wf = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = json_call(
        &h.app,
        "PUT",
        &format!("/api/workflows/{wf}"),
        Some(json!({"graph": graph})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, run) = json_call(&h.app, "POST", &format!("/api/workflows/{wf}/runs"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let run_id = run["runId"].as_str().unwrap().to_owned();
    for _ in 0..200 {
        let (_, detail) = json_call(&h.app, "GET", &format!("/api/runs/{run_id}"), None).await;
        if detail["status"] == "succeeded" {
            return run_id;
        }
        assert_ne!(detail["status"], "failed", "{detail}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run did not finish");
}

#[sqlx::test(migrations = "./migrations")]
async fn downloads_artifacts_with_headers(pool: PgPool) {
    let h = harness(pool);
    let run = completed_run(&h).await;

    let md = download(&h.app, &format!("/api/runs/{run}/artifacts/x/md")).await;
    assert_eq!(md.status, StatusCode::OK);
    assert_eq!(md.body, b"x:md");
    assert_eq!(md.headers["content-type"], "text/markdown; charset=utf-8");
    assert_eq!(md.headers["content-length"], "4");
    assert_eq!(
        md.headers["content-disposition"],
        "inline; filename=\"md.md\"; filename*=UTF-8''md.md"
    );

    // Explicit index, list socket, and type-specific content types.
    let img = download(
        &h.app,
        &format!("/api/runs/{run}/artifacts/x/images?index=0"),
    )
    .await;
    assert_eq!(img.status, StatusCode::OK);
    assert_eq!(img.body, b"x:images");
    assert_eq!(img.headers["content-type"], "image/jpeg");
    assert_eq!(
        img.headers["content-disposition"],
        "inline; filename=\"0000.jpg\"; filename*=UTF-8''0000.jpg"
    );
    let json_file = download(&h.app, &format!("/api/runs/{run}/artifacts/x/json")).await;
    assert_eq!(json_file.headers["content-type"], "application/json");
    let csv = download(
        &h.app,
        &format!("/api/runs/{run}/artifacts/x/tables?index=0"),
    )
    .await;
    assert_eq!(csv.headers["content-type"], "text/csv; charset=utf-8");
    let pdf = download(&h.app, &format!("/api/runs/{run}/artifacts/p/pdf")).await;
    assert_eq!(pdf.headers["content-type"], "application/pdf");

    // URLs advertised by the run detail are downloadable as-is.
    let (_, detail) = json_call(&h.app, "GET", &format!("/api/runs/{run}"), None).await;
    let url = detail["nodes"][2]["outputs"][0]["files"][0]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let via_url = download(&h.app, &url).await;
    assert_eq!(via_url.status, StatusCode::OK);
    assert_eq!(via_url.body, b"x:md");
}

#[sqlx::test(migrations = "./migrations")]
async fn missing_artifacts_are_404_and_bad_index_is_400(pool: PgPool) {
    let h = harness(pool);
    let run = completed_run(&h).await;

    for uri in [
        format!("/api/runs/{run}/artifacts/x/images?index=1"), // out of range
        format!("/api/runs/{run}/artifacts/x/md?index=5"),
        format!("/api/runs/{run}/artifacts/x/nope"), // unknown socket
        format!("/api/runs/{run}/artifacts/ghost/md"), // unknown node
        format!("/api/runs/{}/artifacts/x/md", Uuid::new_v4()), // unknown run
    ] {
        let d = download(&h.app, &uri).await;
        assert_eq!(d.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(error_code(&d), "NOT_FOUND");
    }
    for uri in [
        format!("/api/runs/{run}/artifacts/x/md?index=-1"),
        format!("/api/runs/{run}/artifacts/x/md?index=abc"),
        format!("/api/runs/{run}/artifacts/x/md?index=1.5"),
        "/api/runs/not-a-uuid/artifacts/x/md".to_owned(),
    ] {
        let d = download(&h.app, &uri).await;
        assert_eq!(d.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(error_code(&d), "BAD_REQUEST");
    }
}

/// Inserts a run whose node outputs point at arbitrary paths.
async fn run_with_output(
    h: &Harness,
    kind: &str,
    socket: &str,
    data_type: Value,
    path: &str,
) -> Uuid {
    let wf: Uuid = sqlx::query_scalar("INSERT INTO workflows (name) VALUES ('w') RETURNING id")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO runs (workflow_id, status, graph_snapshot) VALUES ($1, 'succeeded', '{}') RETURNING id",
    )
    .bind(wf).fetch_one(&h.pool).await.unwrap();
    let outputs = json!({ socket: { "dataType": data_type, "paths": [path] } });
    sqlx::query("INSERT INTO node_runs (run_id, node_id, kind, status, outputs) VALUES ($1, 'n', $2, 'succeeded', $3)")
        .bind(run).bind(kind).bind(outputs).execute(&h.pool).await.unwrap();
    run
}

#[sqlx::test(migrations = "./migrations")]
async fn paths_outside_the_data_dir_are_not_served(pool: PgPool) {
    let h = harness(pool);
    let md = json!({"scalar": "MD", "list": false});

    // Absolute path outside DATA_DIR.
    let secret = h
        .data
        .parent()
        .unwrap()
        .join(format!("secret-{}.md", Uuid::new_v4()));
    std::fs::write(&secret, "top secret").unwrap();
    let run = run_with_output(&h, "analyze", "md", md.clone(), secret.to_str().unwrap()).await;
    let d = download(&h.app, &format!("/api/runs/{run}/artifacts/n/md")).await;
    assert_eq!(d.status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&d), "NOT_FOUND");

    // `..` traversal that starts inside DATA_DIR.
    let traversal = format!(
        "{}/runs/../../{}",
        h.data.display(),
        secret.file_name().unwrap().to_str().unwrap()
    );
    let run = run_with_output(&h, "analyze", "md", md.clone(), &traversal).await;
    let d = download(&h.app, &format!("/api/runs/{run}/artifacts/n/md")).await;
    assert_eq!(d.status, StatusCode::NOT_FOUND);

    // Non-existent file and directories are 404 too.
    let missing = format!("{}/runs/none.md", h.data.display());
    let run = run_with_output(&h, "analyze", "md", md.clone(), &missing).await;
    assert_eq!(
        download(&h.app, &format!("/api/runs/{run}/artifacts/n/md"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let run = run_with_output(&h, "analyze", "md", md, h.data.to_str().unwrap()).await;
    assert_eq!(
        download(&h.app, &format!("/api/runs/{run}/artifacts/n/md"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let _ = std::fs::remove_file(secret);
}

#[sqlx::test(migrations = "./migrations")]
async fn bundle_paths_inside_the_data_dir_are_served(pool: PgPool) {
    let h = harness(pool);
    let bundle_file = h.data.join("bundles").join(BUNDLE_ID).join("file.pdf");
    std::fs::create_dir_all(bundle_file.parent().unwrap()).unwrap();
    std::fs::write(&bundle_file, b"%PDF-1.7 hello").unwrap();

    let run = run_with_output(
        &h,
        "bundle",
        "files",
        json!({"scalar": "PDF", "list": true}),
        bundle_file.to_str().unwrap(),
    )
    .await;
    let d = download(
        &h.app,
        &format!("/api/runs/{run}/artifacts/n/files?index=0"),
    )
    .await;
    assert_eq!(d.status, StatusCode::OK);
    assert_eq!(d.body, b"%PDF-1.7 hello");
    assert_eq!(d.headers["content-type"], "application/pdf");
}
