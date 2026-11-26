use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::UnimplementedRunner;
use dochestra_engine::state::AppState;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const BUNDLE_ID: &str = "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f";

fn app(pool: PgPool) -> Router {
    let config =
        Config::from_lookup(|k| (k == "DATABASE_URL").then(|| "unused".to_owned())).unwrap();
    build_app(AppState::new(
        pool,
        config,
        reqwest::Client::new(),
        Arc::new(UnimplementedRunner),
    ))
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
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

async fn workflow_with_graph(app: &Router, graph: Value) -> String {
    let (_, wf) = call(app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let id = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = call(
        app,
        "PUT",
        &format!("/api/workflows/{id}"),
        Some(json!({"graph": graph})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    id
}

fn pipeline(output_on_extract: bool) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": BUNDLE_ID}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), output_on_extract),
        ],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn valid_graph_reports_ok(pool: PgPool) {
    let app = app(pool);
    let id = workflow_with_graph(&app, pipeline(true)).await;
    let (status, report) = call(&app, "POST", &format!("/api/workflows/{id}/validate"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report, json!({"ok": true, "issues": []}));
}

#[sqlx::test(migrations = "./migrations")]
async fn graph_without_output_node_reports_issue(pool: PgPool) {
    let app = app(pool);
    let id = workflow_with_graph(&app, pipeline(false)).await;
    let (status, report) = call(&app, "POST", &format!("/api/workflows/{id}/validate"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report,
        json!({"ok": false, "issues": [{"code": "NO_OUTPUT_NODE"}]})
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn incomplete_saved_graph_reports_unconnected_input_and_ignores_non_target_nodes(
    pool: PgPool,
) {
    let app = app(pool);
    // Saving incomplete graphs is allowed; validation reports what is missing.
    let graph = json!({
        "nodes": [node("b", "bundle", json!({"bundleId": null}), false), node("x", "extract", json!({}), true)],
        "edges": []
    });
    let id = workflow_with_graph(&app, graph).await;
    let (_, report) = call(&app, "POST", &format!("/api/workflows/{id}/validate"), None).await;
    assert_eq!(report["ok"], false);
    assert_eq!(
        report["issues"],
        json!([{"code": "UNCONNECTED_INPUT", "nodeId": "x", "socketId": "pdf"}])
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_id_is_404_and_bad_id_is_400(pool: PgPool) {
    let app = app(pool);
    let uri = format!("/api/workflows/{}/validate", uuid::Uuid::new_v4());
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
    let (status, body) = call(&app, "POST", "/api/workflows/nope/validate", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}
