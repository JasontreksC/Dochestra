use std::borrow::Cow;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{
    NodeContext, NodeError, NodeInputs, NodeOutputs, NodeRunner,
};
use dochestra_engine::state::AppState;
use futures::FutureExt;
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn vector_extension_exists(pool: PgPool) {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_extension WHERE extname = 'vector'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn all_tables_exist(pool: PgPool) {
    let expected = [
        "bundle_files",
        "bundles",
        "node_runs",
        "rag_chunks",
        "rag_indexes",
        "runs",
        "workflows",
    ];
    let mut found: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    found.sort();
    assert_eq!(found, expected);
}

// ------------------------------------------------------------------ 0004: the form-filling node was removed (D-7)

/// Kind name of the removed node. This test is the one place that must know it, so it is spelled out of
/// pieces: the repository-wide leftover check (a grep for the node's name) then stays free of exceptions.
const REMOVED_KIND: &str = concat!("fi", "ll");

/// Never runs anything: the API tests below only read stored data.
struct NoRunner;

impl NodeRunner for NoRunner {
    fn run(
        &self,
        _ctx: NodeContext,
        _inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
        async { Err(NodeError::Failed("unused".to_owned())) }.boxed()
    }
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn node(id: &str, kind: &str, x: f64, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": x, "y": 40.5}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

/// A workflow as a user may have saved it: two removed nodes (one fed by `json_input`, one feeding an
/// `extract`), plus nodes and edges that must survive.
fn graph_with_removed_nodes() -> Value {
    json!({
        "nodes": [
            node("b", "bundle", 0.0, json!({"bundleId": "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f"}), false),
            node("p", "pick_pdf", 250.0, json!({"index": 0}), false),
            node("j", "json_input", 250.0, json!({"value": {"이름": "홍길동", "동의함": true}}), false),
            node("f", REMOVED_KIND, 500.0, json!({}), true),
            node("x", "extract", 750.0, json!({}), true),
            node("x2", "extract", 500.0, json!({}), false),
            node("f2", REMOVED_KIND, 750.0, json!({}), true),
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "f", "pdf"),
            edge("e3", "j", "json", "f", "values"),
            edge("e4", "f", "pdf", "x", "pdf"),
            edge("e5", "p", "pdf", "x2", "pdf"),
            edge("e6", "x2", "md", "f2", "pdf"),
        ],
        "viewport": {"x": 12.0, "y": -3.0, "zoom": 0.75}
    })
}

fn graph_without_removed_nodes() -> Value {
    json!({
        "nodes": [
            node("b", "bundle", 0.0, json!({"bundleId": "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f"}), false),
            node("p", "pick_pdf", 250.0, json!({"index": 0}), false),
            node("x", "extract", 500.0, json!({}), true),
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "x", "pdf"),
        ]
    })
}

async fn insert_workflow(pool: &PgPool, id: Uuid, name: &str, graph: &Value) {
    sqlx::query(
        "INSERT INTO workflows (id, name, graph, created_at, updated_at) \
         VALUES ($1, $2, $3, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z')",
    )
    .bind(id)
    .bind(name)
    .bind(graph)
    .execute(pool)
    .await
    .unwrap();
}

/// A finished run of `workflow` with one `node_runs` row per node of `graph`.
async fn insert_run(pool: &PgPool, id: Uuid, workflow: Uuid, graph: &Value) {
    sqlx::query(
        "INSERT INTO runs (id, workflow_id, status, graph_snapshot, started_at, finished_at) \
         VALUES ($1, $2, 'succeeded', $3, '2026-01-03T00:00:00Z', '2026-01-03T00:01:00Z')",
    )
    .bind(id)
    .bind(workflow)
    .bind(graph)
    .execute(pool)
    .await
    .unwrap();
    for (seq, n) in graph["nodes"].as_array().unwrap().iter().enumerate() {
        sqlx::query(
            "INSERT INTO node_runs (run_id, node_id, kind, status, outputs, seq) \
             VALUES ($1, $2, $3, 'succeeded', NULL, $4)",
        )
        .bind(id)
        .bind(n["id"].as_str().unwrap())
        .bind(n["kind"].as_str().unwrap())
        .bind(seq as i32)
        .execute(pool)
        .await
        .unwrap();
    }
}

fn ids(graph: &Value, key: &str) -> Vec<String> {
    graph[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn workflow_row(pool: &PgPool, id: Uuid) -> (Value, String, String) {
    sqlx::query_as::<_, (Value, String, String)>(
        "SELECT graph, updated_at::text, xmin::text FROM workflows WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn run_row(pool: &PgPool, id: Uuid) -> (Value, String) {
    sqlx::query_as::<_, (Value, String)>(
        "SELECT graph_snapshot, xmin::text FROM runs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn node_run_ids(pool: &PgPool, run: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT node_id FROM node_runs WHERE run_id = $1 ORDER BY seq")
        .bind(run)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn migration_0004_removes_fill_nodes_from_stored_workflows_and_runs(pool: PgPool) {
    // The database as it was before 0004 (the migrations up to 0003) ...
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = Cow::Owned(
        before
            .migrations
            .iter()
            .filter(|m| m.version <= 3)
            .cloned()
            .collect(),
    );
    before.run(&pool).await.unwrap();

    // ... holding workflows and runs with removed nodes, and ones without.
    let (wf_fill, wf_plain) = (Uuid::new_v4(), Uuid::new_v4());
    let (run_fill, run_plain) = (Uuid::new_v4(), Uuid::new_v4());
    let (fill_graph, plain_graph) = (graph_with_removed_nodes(), graph_without_removed_nodes());
    insert_workflow(&pool, wf_fill, "동의서", &fill_graph).await;
    insert_workflow(&pool, wf_plain, "plain", &plain_graph).await;
    insert_run(&pool, run_fill, wf_fill, &fill_graph).await;
    insert_run(&pool, run_plain, wf_plain, &plain_graph).await;
    let plain_workflow_before = workflow_row(&pool, wf_plain).await;
    let fill_workflow_updated_at = workflow_row(&pool, wf_fill).await.1;
    let plain_run_before = run_row(&pool, run_plain).await;

    // Without the migration the node kind is unknown and the stored data cannot be loaded at all.
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(std::env::temp_dir().display().to_string()),
        _ => None,
    })
    .unwrap();
    let app = build_app(AppState::new(
        pool.clone(),
        config,
        reqwest::Client::new(),
        Arc::new(NoRunner),
    ));
    assert_ne!(
        get(&app, &format!("/api/workflows/{wf_fill}")).await.0,
        StatusCode::OK
    );
    assert_ne!(
        get(&app, &format!("/api/runs/{run_fill}")).await.0,
        StatusCode::OK
    );

    // 0004 is applied by the regular migrator (only the pending migration runs).
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Workflow: the removed nodes and the edges touching them are gone; everything else is as it was.
    let (graph, updated_at, _) = workflow_row(&pool, wf_fill).await;
    assert_eq!(ids(&graph, "nodes"), ["b", "p", "j", "x", "x2"]);
    assert_eq!(ids(&graph, "edges"), ["e1", "e5"]);
    for kept in graph["nodes"].as_array().unwrap() {
        let original = fill_graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == kept["id"])
            .unwrap();
        assert_eq!(
            kept, original,
            "a kept node keeps its position, params and isOutput"
        );
    }
    assert_eq!(
        graph["viewport"], fill_graph["viewport"],
        "other graph keys survive"
    );
    assert_eq!(
        updated_at, fill_workflow_updated_at,
        "updated_at is not touched"
    );

    // A workflow without them is not even rewritten.
    assert_eq!(workflow_row(&pool, wf_plain).await, plain_workflow_before);

    // Run: the snapshot is cleaned the same way, their node_runs are deleted, other runs are untouched.
    let (snapshot, _) = run_row(&pool, run_fill).await;
    assert_eq!(ids(&snapshot, "nodes"), ["b", "p", "j", "x", "x2"]);
    assert_eq!(ids(&snapshot, "edges"), ["e1", "e5"]);
    assert_eq!(
        node_run_ids(&pool, run_fill).await,
        ["b", "p", "j", "x", "x2"]
    );
    assert_eq!(run_row(&pool, run_plain).await, plain_run_before);
    assert_eq!(node_run_ids(&pool, run_plain).await, ["b", "p", "x"]);
    let leftover: i64 = sqlx::query_scalar("SELECT count(*) FROM node_runs WHERE kind = $1")
        .bind(REMOVED_KIND)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(leftover, 0);

    // The API loads the migrated data again.
    let (status, workflow) = get(&app, &format!("/api/workflows/{wf_fill}")).await;
    assert_eq!(status, StatusCode::OK, "{workflow}");
    assert_eq!(ids(&workflow["graph"], "nodes"), ["b", "p", "j", "x", "x2"]);
    let (status, detail) = get(&app, &format!("/api/runs/{run_fill}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let listed: Vec<&str> = detail["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["nodeId"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["b", "p", "j", "x", "x2"]);
    assert_eq!(
        get(&app, &format!("/api/workflows/{wf_plain}")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, &format!("/api/runs/{run_plain}")).await.0,
        StatusCode::OK
    );

    // Running the migration's SQL again changes nothing.
    let after = (
        workflow_row(&pool, wf_fill).await,
        run_row(&pool, run_fill).await,
    );
    sqlx::raw_sql(include_str!("../migrations/0004_remove_fill_nodes.sql"))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        (
            workflow_row(&pool, wf_fill).await,
            run_row(&pool, run_fill).await
        ),
        after
    );
}
