use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::catalog::spec;
use dochestra_engine::config::Config;
use dochestra_engine::db::runs as db_runs;
use dochestra_engine::executor::events::EventHub;
use dochestra_engine::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs, NodeRunner,
};
use dochestra_engine::state::AppState;
use dochestra_engine::storage::{list_output_path, single_output_path};
use dochestra_engine::types::{NodeStatus, RunEvent, RunStatus};
use futures::FutureExt;
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tower::ServiceExt;
use uuid::Uuid;

const BUNDLE_ID: &str = "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f";

/// Test runner: nodes wait for a permit from `gate` (if any), may fail by node id, and
/// otherwise write dummy files for every catalog output socket.
struct TestRunner {
    gate: Option<Arc<Semaphore>>,
    fail_nodes: Vec<String>,
    /// Sleep before every node finishes.
    delay: Duration,
}

impl NodeRunner for TestRunner {
    fn run(
        &self,
        ctx: NodeContext,
        _inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
        let gate = self.gate.clone();
        let fail = self.fail_nodes.contains(&ctx.node_id);
        let delay = self.delay;
        async move {
            tokio::time::sleep(delay).await;
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            if fail {
                return Err(NodeError::Failed("boom".to_owned()));
            }
            let mut outputs = NodeOutputs::new();
            for socket in &spec(ctx.kind).outputs {
                let scalar = socket.data_type.scalar;
                let path: PathBuf = if socket.data_type.list {
                    let p = list_output_path(&ctx.out_dir, &socket.id, scalar, 0);
                    tokio::fs::create_dir_all(p.parent().unwrap())
                        .await
                        .unwrap();
                    p
                } else {
                    single_output_path(&ctx.out_dir, &socket.id, scalar)
                };
                tokio::fs::write(&path, b"data").await.unwrap();
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
    events: EventHub,
    _data: TempDir,
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn harness(pool: PgPool, runner: TestRunner) -> Harness {
    let data = TempDir(std::env::temp_dir().join(format!("dochestra-runs-{}", Uuid::new_v4())));
    std::fs::create_dir_all(&data.0).unwrap();
    let dir = data.0.to_string_lossy().into_owned();
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        _ => None,
    })
    .unwrap();
    let state = AppState::new(
        pool.clone(),
        config,
        reqwest::Client::new(),
        Arc::new(runner),
    );
    let events = state.events.clone();
    let app = build_app(state);
    Harness {
        app,
        pool,
        events,
        _data: data,
    }
}

fn plain_runner() -> TestRunner {
    TestRunner {
        gate: None,
        fail_nodes: vec![],
        delay: Duration::ZERO,
    }
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn pipeline(with_output: bool) -> Value {
    json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": BUNDLE_ID}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("x", "extract", json!({}), with_output),
        ],
        "edges": [edge("e1", "b", "files", "p", "files"), edge("e2", "p", "pdf", "x", "pdf")]
    })
}

async fn workflow(app: &Router, graph: Value) -> String {
    let (_, wf) = send(app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let id = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = send(
        app,
        "PUT",
        &format!("/api/workflows/{id}"),
        Some(json!({"graph": graph})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    id
}

async fn start_run(app: &Router, workflow_id: &str) -> String {
    let (status, body) = send(
        app,
        "POST",
        &format!("/api/workflows/{workflow_id}/runs"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["runId"].as_str().unwrap().to_owned()
}

/// Opens the SSE stream and returns the response (headers) without consuming the body.
async fn open_events(app: &Router, run_id: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::get(format!("/api/runs/{run_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Reads an SSE body to its end and parses every `data:` payload.
async fn collect_events(resp: axum::response::Response) -> Vec<Value> {
    let bytes = tokio::time::timeout(Duration::from_secs(20), resp.into_body().collect())
        .await
        .expect("SSE stream did not end")
        .unwrap()
        .to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    text.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| serde_json::from_str(d.trim()).unwrap())
        .collect()
}

async fn wait_finished(app: &Router, run_id: &str) -> Value {
    for _ in 0..200 {
        let (_, detail) = send(app, "GET", &format!("/api/runs/{run_id}"), None).await;
        if detail["status"] != "running" {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {run_id} did not finish");
}

fn node_status<'a>(detail: &'a Value, node_id: &str) -> &'a str {
    detail["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["nodeId"] == node_id)
        .unwrap_or_else(|| panic!("no node {node_id}"))["status"]
        .as_str()
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn run_streams_snapshot_updates_and_finishes(pool: PgPool) {
    let gate = Arc::new(Semaphore::new(0));
    let h = harness(
        pool,
        TestRunner {
            gate: Some(gate.clone()),
            fail_nodes: vec![],
            delay: Duration::ZERO,
        },
    );
    let wf = workflow(&h.app, pipeline(true)).await;
    let run_id = start_run(&h.app, &wf).await;

    let resp = open_events(&h.app, &run_id).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    assert_eq!(resp.headers()["x-accel-buffering"], "no");
    gate.add_permits(100);
    let events = collect_events(resp).await;

    // First event: snapshot of all three planned nodes, not yet finished.
    assert_eq!(events[0]["type"], "snapshot");
    let snap_nodes = events[0]["run"]["nodes"].as_array().unwrap();
    assert_eq!(snap_nodes.len(), 3);
    assert!(
        snap_nodes
            .iter()
            .all(|n| ["pending", "running"].contains(&n["status"].as_str().unwrap()))
    );
    assert_eq!(events[0]["run"]["status"], "running");

    // Last event: run_finished(succeeded).
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "run_finished", "status": "succeeded"})
    );

    // Each node reaches succeeded via node_status events; `running` is seen either in the
    // snapshot or as an event, and never after the terminal state.
    for id in ["b", "p", "x"] {
        let seq: Vec<&str> = events
            .iter()
            .filter(|e| e["type"] == "node_status" && e["nodeId"] == id)
            .map(|e| e["status"].as_str().unwrap())
            .collect();
        assert_eq!(seq.last(), Some(&"succeeded"), "{id}: {seq:?}");
        assert_eq!(seq.iter().filter(|s| **s == "succeeded").count(), 1);
        let snap_status = snap_nodes.iter().find(|n| n["nodeId"] == id).unwrap()["status"]
            .as_str()
            .unwrap();
        assert!(
            seq.contains(&"running") || snap_status == "running",
            "{id}: {seq:?}"
        );
    }
    let x_done = events
        .iter()
        .find(|e| e["type"] == "node_status" && e["nodeId"] == "x" && e["status"] == "succeeded")
        .unwrap();
    assert_eq!(x_done["error"], Value::Null);
    let sockets: Vec<&str> = x_done["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["socketId"].as_str().unwrap())
        .collect();
    assert_eq!(sockets, ["md", "json", "tables", "images"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn run_detail_has_outputs_with_urls(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf = workflow(&h.app, pipeline(true)).await;
    let run_id = start_run(&h.app, &wf).await;
    let detail = wait_finished(&h.app, &run_id).await;

    assert_eq!(detail["status"], "succeeded");
    assert_eq!(detail["id"], run_id.as_str());
    assert_eq!(detail["workflowId"], wf.as_str());
    assert!(detail["finishedAt"].is_string());
    let ids: Vec<&str> = detail["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["nodeId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["b", "p", "x"]); // plan (seq) order
    for n in detail["nodes"].as_array().unwrap() {
        assert_eq!(n["status"], "succeeded");
        assert!(n["startedAt"].is_string() && n["finishedAt"].is_string());
        assert_eq!(n["error"], Value::Null);
    }
    let x = &detail["nodes"][2];
    let outputs = x["outputs"].as_array().unwrap();
    assert_eq!(outputs[0]["socketId"], "md");
    assert_eq!(
        outputs[0]["dataType"],
        json!({"scalar": "MD", "list": false})
    );
    assert_eq!(outputs[0]["files"][0]["fileName"], "md.md");
    assert_eq!(
        outputs[0]["files"][0]["url"],
        format!("/api/runs/{run_id}/artifacts/x/md?index=0")
    );
    assert_eq!(outputs[3]["socketId"], "images");
    assert_eq!(outputs[3]["files"][0]["fileName"], "0000.jpg");
    assert_eq!(
        outputs[3]["files"][0]["url"],
        format!("/api/runs/{run_id}/artifacts/x/images?index=0")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn run_without_output_node_is_422_and_creates_nothing(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf = workflow(&h.app, pipeline(false)).await;
    let (status, body) = send(&h.app, "POST", &format!("/api/workflows/{wf}/runs"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(body["error"]["issues"][0]["code"], "NO_OUTPUT_NODE");
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(runs, 0);
    let (status, _) = send(
        &h.app,
        "POST",
        &format!("/api/workflows/{}/runs", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn failing_node_fails_run_and_skips_downstream(pool: PgPool) {
    let gate = Arc::new(Semaphore::new(0));
    let h = harness(
        pool,
        TestRunner {
            gate: Some(gate.clone()),
            fail_nodes: vec!["p".to_owned()],
            delay: Duration::ZERO,
        },
    );
    let wf = workflow(&h.app, pipeline(true)).await;
    let run_id = start_run(&h.app, &wf).await;
    let resp = open_events(&h.app, &run_id).await;
    gate.add_permits(100);
    let events = collect_events(resp).await;

    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "run_finished", "status": "failed"})
    );
    let status_of = |id: &str| -> Vec<&str> {
        events
            .iter()
            .filter(|e| e["type"] == "node_status" && e["nodeId"] == id)
            .map(|e| e["status"].as_str().unwrap())
            .collect()
    };
    assert_eq!(status_of("p").last(), Some(&"failed"));
    assert_eq!(status_of("x"), ["skipped"]);
    let failed = events
        .iter()
        .find(|e| e["type"] == "node_status" && e["status"] == "failed")
        .unwrap();
    assert_eq!(failed["error"], "boom");

    let (_, detail) = send(&h.app, "GET", &format!("/api/runs/{run_id}"), None).await;
    assert_eq!(detail["status"], "failed");
    assert_eq!(node_status(&detail, "b"), "succeeded");
    assert_eq!(node_status(&detail, "p"), "failed");
    assert_eq!(node_status(&detail, "x"), "skipped");
    assert_eq!(detail["nodes"][1]["error"], "boom");
}

#[sqlx::test(migrations = "./migrations")]
async fn sse_after_completion_returns_snapshot_and_finish(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf = workflow(&h.app, pipeline(true)).await;
    let run_id = start_run(&h.app, &wf).await;
    wait_finished(&h.app, &run_id).await;

    let events = collect_events(open_events(&h.app, &run_id).await).await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["type"], "snapshot");
    assert_eq!(events[0]["run"]["status"], "succeeded");
    assert!(
        events[0]["run"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["status"] == "succeeded")
    );
    assert_eq!(
        events[1],
        json!({"type": "run_finished", "status": "succeeded"})
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn runs_are_listed_newest_first(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf = workflow(&h.app, pipeline(true)).await;
    let first = start_run(&h.app, &wf).await;
    wait_finished(&h.app, &first).await;
    let second = start_run(&h.app, &wf).await;
    wait_finished(&h.app, &second).await;

    let (status, list) = send(&h.app, "GET", &format!("/api/workflows/{wf}/runs"), None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [second.as_str(), first.as_str()]);
    assert_eq!(list[0]["status"], "succeeded");
    assert!(list[0]["startedAt"].is_string() && list[0]["finishedAt"].is_string());
    assert!(list[0].get("nodes").is_none());

    let (status, body) = send(
        &h.app,
        "GET",
        &format!("/api/workflows/{}/runs", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[sqlx::test(migrations = "./migrations")]
async fn orphaned_runs_are_cleaned_up(pool: PgPool) {
    let wf: Uuid = sqlx::query_scalar("INSERT INTO workflows (name) VALUES ('w') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let orphan: Uuid = sqlx::query_scalar(
        "INSERT INTO runs (workflow_id, status, graph_snapshot) VALUES ($1, 'running', '{}') RETURNING id",
    )
    .bind(wf)
    .fetch_one(&pool)
    .await
    .unwrap();
    let done: Uuid = sqlx::query_scalar(
        "INSERT INTO runs (workflow_id, status, graph_snapshot, finished_at) VALUES ($1, 'succeeded', '{}', now()) RETURNING id",
    )
    .bind(wf)
    .fetch_one(&pool)
    .await
    .unwrap();
    for (run, node, status, seq) in [
        (orphan, "a", "succeeded", 0),
        (orphan, "b", "running", 1),
        (orphan, "c", "pending", 2),
        (done, "d", "succeeded", 0),
    ] {
        sqlx::query("INSERT INTO node_runs (run_id, node_id, kind, status, seq) VALUES ($1, $2, 'extract', $3, $4)")
            .bind(run)
            .bind(node)
            .bind(status)
            .bind(seq)
            .execute(&pool)
            .await
            .unwrap();
    }

    assert_eq!(db_runs::fail_orphaned_runs(&pool).await.unwrap(), 1);
    assert_eq!(db_runs::fail_orphaned_runs(&pool).await.unwrap(), 0); // idempotent

    let run: (String, bool) =
        sqlx::query_as("SELECT status, finished_at IS NOT NULL FROM runs WHERE id = $1")
            .bind(orphan)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(run, ("failed".to_owned(), true));
    let nodes: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT node_id, status, error FROM node_runs WHERE run_id = $1 ORDER BY seq",
    )
    .bind(orphan)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        nodes,
        vec![
            ("a".to_owned(), "succeeded".to_owned(), None),
            (
                "b".to_owned(),
                "failed".to_owned(),
                Some("엔진 재시작으로 중단됨".to_owned())
            ),
            ("c".to_owned(), "skipped".to_owned(), None),
        ]
    );
    let untouched: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(done)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(untouched, "succeeded");
    let d: String = sqlx::query_scalar("SELECT status FROM node_runs WHERE run_id = $1")
        .bind(done)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(d, "succeeded");
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_run_is_404_json(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let id = Uuid::new_v4();
    let (status, body) = send(&h.app, "GET", &format!("/api/runs/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");

    let resp = open_events(&h.app, &id.to_string()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "NOT_FOUND");

    let (status, body) = send(&h.app, "GET", "/api/runs/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn runs_are_cascade_deleted_with_their_workflow(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf = workflow(&h.app, pipeline(true)).await;
    let run_id = start_run(&h.app, &wf).await;
    wait_finished(&h.app, &run_id).await;
    let (status, _) = send(&h.app, "DELETE", &format!("/api/workflows/{wf}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&h.app, "GET", &format!("/api/runs/{run_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Summarizes an SSE event list as `(snapshot node statuses, then node_status events)`.
fn status_events(events: &[Value]) -> Vec<(String, String)> {
    events
        .iter()
        .filter(|e| e["type"] == "node_status")
        .map(|e| {
            (
                e["nodeId"].as_str().unwrap().to_owned(),
                e["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Reproduces the race behind the e2e failure deterministically: broadcasts that were already
/// persisted (and therefore are part of the snapshot) arrive after the snapshot.
#[sqlx::test(migrations = "./migrations")]
async fn stale_and_duplicate_broadcasts_are_not_forwarded(pool: PgPool) {
    let h = harness(pool, plain_runner());
    let wf: Uuid = sqlx::query_scalar("INSERT INTO workflows (name) VALUES ('w') RETURNING id")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO runs (workflow_id, status, graph_snapshot) VALUES ($1, 'running', '{}') RETURNING id",
    )
    .bind(wf)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    for (node, status, seq) in [
        ("a", "succeeded", 0),
        ("b", "running", 1),
        ("c", "pending", 2),
        ("d", "pending", 3),
    ] {
        sqlx::query("INSERT INTO node_runs (run_id, node_id, kind, status, seq) VALUES ($1, $2, 'extract', $3, $4)")
            .bind(run).bind(node).bind(status).bind(seq)
            .execute(&h.pool).await.unwrap();
    }
    h.events.create(run);

    // The handler subscribes and builds the snapshot (a: succeeded, b: running, c/d: pending).
    let resp = open_events(&h.app, &run.to_string()).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let node_status = |node: &str, status: NodeStatus| RunEvent::NodeStatus {
        node_id: node.to_owned(),
        status,
        error: None,
        outputs: vec![],
    };
    for ev in [
        node_status("a", NodeStatus::Running), // stale: already succeeded in the snapshot
        node_status("a", NodeStatus::Succeeded), // duplicate of the snapshot state
        node_status("a", NodeStatus::Failed),  // conflicting terminal state
        node_status("b", NodeStatus::Running), // duplicate
        node_status("b", NodeStatus::Pending), // regression
        node_status("b", NodeStatus::Succeeded), // real progress
        node_status("b", NodeStatus::Succeeded), // repeated
        node_status("c", NodeStatus::Running), // real progress
        node_status("c", NodeStatus::Running), // repeated
        node_status("c", NodeStatus::Failed),  // real progress
        node_status("d", NodeStatus::Skipped), // pending -> terminal is fine
        RunEvent::RunFinished {
            status: RunStatus::Failed,
        },
    ] {
        h.events.send(run, ev);
    }
    let events = collect_events(resp).await;

    assert_eq!(events[0]["type"], "snapshot");
    assert_eq!(
        status_events(&events),
        [
            ("b".to_owned(), "succeeded".to_owned()),
            ("c".to_owned(), "running".to_owned()),
            ("c".to_owned(), "failed".to_owned()),
            ("d".to_owned(), "skipped".to_owned()),
        ]
    );
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "run_finished", "status": "failed"})
    );
    assert_eq!(events.len(), 6);
}

/// Connects to a running workflow at many different moments; every connection must see
/// per-node monotonic statuses and nothing after a node's terminal status.
#[sqlx::test(migrations = "./migrations")]
async fn every_connection_sees_monotonic_node_statuses(pool: PgPool) {
    let h = harness(
        pool,
        TestRunner {
            gate: None,
            fail_nodes: vec![],
            delay: Duration::from_millis(25),
        },
    );
    let wf = workflow(&h.app, pipeline(true)).await;

    for attempt in 0..20u64 {
        let run_id = start_run(&h.app, &wf).await;
        tokio::time::sleep(Duration::from_millis(attempt * 6)).await;
        let events = collect_events(open_events(&h.app, &run_id).await).await;

        assert_eq!(events[0]["type"], "snapshot", "attempt {attempt}");
        let rank = |s: &str| match s {
            "pending" => 0,
            "running" => 1,
            _ => 2,
        };
        let mut last: std::collections::HashMap<String, i32> = events[0]["run"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["nodeId"].as_str().unwrap().to_owned(),
                    rank(n["status"].as_str().unwrap()),
                )
            })
            .collect();
        for (node, status) in status_events(&events) {
            let r = rank(&status);
            assert!(
                r > last[&node],
                "attempt {attempt}: {node} went {:?} -> {status}\n{events:#?}",
                last[&node]
            );
            last.insert(node, r);
        }
        assert_eq!(
            events.last().unwrap(),
            &json!({"type": "run_finished", "status": "succeeded"}),
            "attempt {attempt}"
        );
        assert!(
            last.values().all(|r| *r == 2),
            "attempt {attempt}: {last:?}"
        );
    }
}
