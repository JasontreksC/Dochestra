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
        serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("non-JSON body: {bytes:?}"))
    };
    (status, json)
}

fn node(id: &str, kind: &str, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 1.5, "y": 2.0}, "params": {}, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

fn valid_graph() -> Value {
    json!({
        "nodes": [node("b", "bundle", false), node("p", "pick_pdf", false), node("x", "extract", true)],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    })
}

fn assert_error(body: &Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "body: {body}");
    assert!(body["error"]["message"].is_string());
}

#[sqlx::test(migrations = "./migrations")]
async fn workflow_lifecycle(pool: PgPool) {
    let app = app(pool);

    let (status, created) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({"name": "  first  "})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "first");
    assert_eq!(created["graph"], json!({"nodes": [], "edges": []}));
    let id = created["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/workflows/{id}");

    let (status, fetched) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, created);

    // List is newest-updated first.
    let (_, second) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({"name": "second"})),
    )
    .await;
    let (status, list) = call(&app, "GET", "/api/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["second", "first"]);
    assert_eq!(list[0]["id"], second["id"]);
    assert!(list[0]["updatedAt"].is_string());
    assert!(list[0].get("graph").is_none());

    // Valid graph round-trips.
    let (status, updated) = call(&app, "PUT", &uri, Some(json!({"graph": valid_graph()}))).await;
    assert_eq!(status, StatusCode::OK, "body: {updated}");
    assert_eq!(updated["graph"], valid_graph());
    let (_, refetched) = call(&app, "GET", &uri, None).await;
    assert_eq!(refetched["graph"], valid_graph());
    assert_eq!(refetched["name"], "first");

    // Updating bumps it to the top of the list.
    let (_, list) = call(&app, "GET", "/api/workflows", None).await;
    assert_eq!(list[0]["id"], id.as_str());

    // Invalid graph is rejected and DB is unchanged.
    let bad = json!({
        "nodes": [node("b", "bundle", false), node("x", "extract", true)],
        "edges": [edge("e1", "b", "files", "x", "pdf")]
    });
    let (status, err) = call(&app, "PUT", &uri, Some(json!({"graph": bad}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_error(&err, "VALIDATION_FAILED");
    assert_eq!(err["error"]["issues"][0]["code"], "TYPE_MISMATCH");
    let (_, unchanged) = call(&app, "GET", &uri, None).await;
    assert_eq!(unchanged["graph"], valid_graph());

    // Name-only update keeps the graph.
    let (status, renamed) = call(&app, "PUT", &uri, Some(json!({"name": "renamed"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "renamed");
    assert_eq!(renamed["graph"], valid_graph());

    // Empty body object is a no-op update.
    let (status, noop) = call(&app, "PUT", &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(noop["name"], "renamed");

    let (status, _) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, err) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
    let (status, err) = call(&app, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_id_returns_404_on_put(pool: PgPool) {
    let app = app(pool);
    let uri = format!("/api/workflows/{}", uuid::Uuid::new_v4());
    let (status, err) = call(&app, "PUT", &uri, Some(json!({"name": "x"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
}

#[sqlx::test(migrations = "./migrations")]
async fn invalid_uuid_path_is_400_json(pool: PgPool) {
    let app = app(pool);
    let (status, err) = call(&app, "GET", "/api/workflows/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn unsupported_kind_is_400_json(pool: PgPool) {
    let app = app(pool);
    let (_, wf) = call(&app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let uri = format!("/api/workflows/{}", wf["id"].as_str().unwrap());
    let graph = json!({"nodes": [node("n", "teleport", false)], "edges": []});
    let (status, err) = call(&app, "PUT", &uri, Some(json!({"graph": graph}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn malformed_json_and_missing_content_type_are_400_json(pool: PgPool) {
    let app = app(pool);
    let req = Request::builder()
        .method("POST")
        .uri("/api/workflows")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_error(&serde_json::from_slice(&bytes).unwrap(), "BAD_REQUEST");

    let (status, err) = call(&app, "POST", "/api/workflows", Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn name_must_be_1_to_100_chars(pool: PgPool) {
    let app = app(pool);
    for name in ["", "   "] {
        let (status, err) = call(&app, "POST", "/api/workflows", Some(json!({"name": name}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_error(&err, "BAD_REQUEST");
    }
    let too_long = "가".repeat(101);
    let (status, _) = call(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({"name": too_long})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let max = "가".repeat(100);
    let (status, created) = call(&app, "POST", "/api/workflows", Some(json!({"name": max}))).await;
    assert_eq!(status, StatusCode::CREATED);

    let uri = format!("/api/workflows/{}", created["id"].as_str().unwrap());
    let (status, err) = call(&app, "PUT", &uri, Some(json!({"name": "  "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}
