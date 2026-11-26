//! Kahn-style parallel workflow executor (RFP 1.6), independent of DB and HTTP.
//!
//! The executor runs an [`ExecutionPlan`] with a [`NodeRunner`], reports every state change to
//! a [`RunObserver`] (awaited before the executor proceeds), reuses each node's result for all
//! consumers, and skips everything downstream of a failed node.

pub mod events;
pub mod observer;
pub mod runner;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use tokio::task::{Id as TaskId, JoinSet};
use tracing::Instrument;
use uuid::Uuid;

pub use self::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs, NodeRunner};
use crate::catalog::spec;
use crate::graph::planner::{ExecutionPlan, PlannedNode};
use crate::storage::run_node_dir;
use crate::types::{NodeKind, NodeStatus, RunStatus};

/// A node state change reported to the [`RunObserver`].
#[derive(Debug, Clone)]
pub struct NodeUpdate {
    pub node_id: String,
    pub status: NodeStatus,
    pub error: Option<String>,
    /// Present only for `Succeeded`.
    pub outputs: Option<NodeOutputs>,
    pub at: DateTime<Utc>,
}

/// Receives node state changes. The executor awaits each call before moving on, so an
/// observer can persist first and broadcast afterwards. `Pending` is never reported.
pub trait RunObserver: Send + Sync + 'static {
    fn node_changed(&self, update: NodeUpdate) -> BoxFuture<'static, ()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: RunStatus,
    /// Final status of every planned node (`Succeeded`, `Failed` or `Skipped`).
    pub nodes: HashMap<String, NodeStatus>,
}

/// Executes `plan`, running nodes whose inputs are ready concurrently.
pub async fn execute(
    plan: &ExecutionPlan,
    run_id: Uuid,
    data_dir: &Path,
    runner: Arc<dyn NodeRunner>,
    observer: Arc<dyn RunObserver>,
) -> RunOutcome {
    let mut sched = Scheduler {
        plan,
        run_id,
        data_dir: data_dir.to_path_buf(),
        runner,
        observer,
        in_degree: plan
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), plan.upstream(&n.id).len()))
            .collect(),
        results: HashMap::new(),
        statuses: HashMap::new(),
        tasks: JoinSet::new(),
        task_nodes: HashMap::new(),
    };

    for node in &plan.nodes {
        if sched.in_degree[node.id.as_str()] == 0 {
            sched.spawn(node);
        }
    }

    while let Some(joined) = sched.tasks.join_next_with_id().await {
        let (task_id, result) = match joined {
            Ok((task_id, result)) => (task_id, result),
            Err(err) => {
                let task_id = err.id();
                let message = if err.is_panic() {
                    format!("노드 실행 중 패닉: {}", panic_message(err.into_panic()))
                } else {
                    "노드 실행이 취소되었습니다".to_owned()
                };
                (task_id, Err(message))
            }
        };
        let Some(node_id) = sched.task_nodes.remove(&task_id) else {
            continue;
        };
        let checked = result.and_then(|outputs| {
            let kind = sched.node(&node_id).kind;
            check_outputs(kind, outputs)
        });
        match checked {
            Ok(outputs) => sched.succeed(&node_id, outputs).await,
            Err(message) => sched.fail(&node_id, message).await,
        }
    }

    let failed = sched.statuses.values().any(|s| *s == NodeStatus::Failed);
    RunOutcome {
        status: if failed {
            RunStatus::Failed
        } else {
            RunStatus::Succeeded
        },
        nodes: sched.statuses,
    }
}

struct Scheduler<'a> {
    plan: &'a ExecutionPlan,
    run_id: Uuid,
    data_dir: PathBuf,
    runner: Arc<dyn NodeRunner>,
    observer: Arc<dyn RunObserver>,
    /// Remaining unfinished upstream nodes per planned node.
    in_degree: HashMap<&'a str, usize>,
    /// Validated outputs of succeeded nodes, shared by all consumers.
    results: HashMap<String, NodeOutputs>,
    /// Terminal status per node.
    statuses: HashMap<String, NodeStatus>,
    tasks: JoinSet<Result<NodeOutputs, String>>,
    task_nodes: HashMap<TaskId, String>,
}

impl Scheduler<'_> {
    fn node(&self, id: &str) -> &PlannedNode {
        self.plan
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| unreachable!("node {id} is not in the plan"))
    }

    async fn report(
        &self,
        node_id: &str,
        status: NodeStatus,
        error: Option<String>,
        outputs: Option<NodeOutputs>,
    ) {
        self.observer
            .node_changed(NodeUpdate {
                node_id: node_id.to_owned(),
                status,
                error,
                outputs,
                at: Utc::now(),
            })
            .await;
    }

    /// Starts a node whose upstream nodes have all succeeded.
    fn spawn(&mut self, node: &PlannedNode) {
        let inputs = self.assemble_inputs(node);
        let runner = Arc::clone(&self.runner);
        let observer = Arc::clone(&self.observer);
        let run_id = self.run_id;
        let data_dir = self.data_dir.clone();
        let node = node.clone();
        let span = tracing::info_span!("node", %run_id, node_id = %node.id, kind = ?node.kind);
        let node_id = node.id.clone();

        let task = async move {
            tracing::info!("node started");
            let inputs = inputs?;
            let out_dir = run_node_dir(&data_dir, run_id, &node.id).map_err(|e| e.to_string())?;
            tokio::fs::create_dir_all(&out_dir)
                .await
                .map_err(|e| format!("출력 디렉토리 생성 실패: {e}"))?;
            observer
                .node_changed(NodeUpdate {
                    node_id: node.id.clone(),
                    status: NodeStatus::Running,
                    error: None,
                    outputs: None,
                    at: Utc::now(),
                })
                .await;
            let ctx = NodeContext {
                run_id,
                node_id: node.id.clone(),
                kind: node.kind,
                params: node.params.clone(),
                out_dir,
            };
            let result = runner.run(ctx, inputs).await.map_err(|err| match err {
                NodeError::Failed(message) => message,
                NodeError::Other(cause) => format!("{cause:#}"),
            });
            match &result {
                Ok(_) => tracing::info!("node finished"),
                Err(message) => tracing::warn!(error = %message, "node failed"),
            }
            result
        }
        .instrument(span);

        let handle = self.tasks.spawn(task);
        self.task_nodes.insert(handle.id(), node_id);
    }

    /// Builds the input map from the upstream nodes' stored outputs.
    fn assemble_inputs(&self, node: &PlannedNode) -> Result<NodeInputs, String> {
        node.inputs
            .iter()
            .map(|binding| {
                self.results
                    .get(&binding.source_node)
                    .and_then(|outputs| outputs.get(&binding.source_socket))
                    .map(|artifact| (binding.socket_id.clone(), artifact.clone()))
                    .ok_or_else(|| {
                        format!(
                            "입력 {} 을(를) 위한 상위 출력 {}.{} 이(가) 없습니다",
                            binding.socket_id, binding.source_node, binding.source_socket
                        )
                    })
            })
            .collect()
    }

    async fn succeed(&mut self, node_id: &str, outputs: NodeOutputs) {
        self.statuses
            .insert(node_id.to_owned(), NodeStatus::Succeeded);
        self.results.insert(node_id.to_owned(), outputs.clone());
        self.report(node_id, NodeStatus::Succeeded, None, Some(outputs))
            .await;

        let plan = self.plan;
        for dependent in plan.dependents(node_id) {
            let Some(degree) = self.in_degree.get_mut(dependent) else {
                continue;
            };
            *degree = degree.saturating_sub(1);
            if *degree == 0 && !self.statuses.contains_key(dependent) {
                let node = self.node(dependent).clone();
                self.spawn(&node);
            }
        }
    }

    async fn fail(&mut self, node_id: &str, message: String) {
        self.statuses.insert(node_id.to_owned(), NodeStatus::Failed);
        self.report(node_id, NodeStatus::Failed, Some(message), None)
            .await;

        // Everything downstream is skipped, reported in plan order.
        let mut downstream: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::from([node_id]);
        while let Some(current) = queue.pop_front() {
            for dependent in self.plan.dependents(current) {
                if downstream.insert(dependent) {
                    queue.push_back(dependent);
                }
            }
        }
        for node in &self.plan.nodes {
            if downstream.contains(node.id.as_str()) && !self.statuses.contains_key(&node.id) {
                self.statuses.insert(node.id.clone(), NodeStatus::Skipped);
                self.report(&node.id, NodeStatus::Skipped, None, None).await;
            }
        }
    }
}

/// Verifies outputs against the catalog contract and keeps only the declared sockets.
fn check_outputs(kind: NodeKind, mut outputs: NodeOutputs) -> Result<NodeOutputs, String> {
    let mut checked = NodeOutputs::new();
    for socket in &spec(kind).outputs {
        let artifact = outputs
            .remove(&socket.id)
            .ok_or_else(|| format!("출력 소켓 {} 누락", socket.id))?;
        if artifact.data_type != socket.data_type {
            return Err(format!(
                "출력 소켓 {} 타입 불일치: 기대 {}, 실제 {}",
                socket.id, socket.data_type, artifact.data_type
            ));
        }
        if !socket.data_type.list && artifact.paths.len() != 1 {
            return Err(format!(
                "출력 소켓 {} 은(는) 파일이 정확히 1개여야 합니다 (실제 {}개)",
                socket.id,
                artifact.paths.len()
            ));
        }
        checked.insert(socket.id.clone(), artifact);
    }
    Ok(checked)
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(s) => *s,
        Err(payload) => payload
            .downcast::<&'static str>()
            .map_or_else(|_| "알 수 없는 패닉".to_owned(), |s| (*s).to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use futures::FutureExt;
    use serde_json::json;

    use super::*;
    use crate::graph::planner::plan_run;
    use crate::storage::{list_output_path, single_output_path};
    use crate::types::{DataType, GraphEdge, GraphNode, Position, WorkflowGraph};

    // ---------------------------------------------------------------- test doubles

    #[derive(Clone)]
    enum Mode {
        Ok,
        Fail(&'static str),
        Panic,
        OmitSocket(&'static str),
        WrongType(&'static str),
        TwoFiles(&'static str),
    }

    #[derive(Clone)]
    struct Behavior {
        delay: Duration,
        mode: Mode,
    }

    #[derive(Default)]
    struct FakeRunner {
        behaviors: HashMap<String, Behavior>,
        calls: Mutex<HashMap<String, usize>>,
        spans: Mutex<HashMap<String, (Instant, Instant)>>,
        received: Mutex<HashMap<String, NodeInputs>>,
    }

    impl FakeRunner {
        fn with(mut self, node_id: &str, delay_ms: u64, mode: Mode) -> Self {
            self.behaviors.insert(
                node_id.to_owned(),
                Behavior {
                    delay: Duration::from_millis(delay_ms),
                    mode,
                },
            );
            self
        }

        fn calls(&self, node_id: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .get(node_id)
                .copied()
                .unwrap_or(0)
        }

        fn span(&self, node_id: &str) -> (Instant, Instant) {
            self.spans.lock().unwrap()[node_id]
        }

        fn inputs_of(&self, node_id: &str) -> NodeInputs {
            self.received.lock().unwrap()[node_id].clone()
        }
    }

    impl NodeRunner for Arc<FakeRunner> {
        fn run(
            &self,
            ctx: NodeContext,
            inputs: NodeInputs,
        ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
            let this = Arc::clone(self);
            async move {
                let started = Instant::now();
                *this
                    .calls
                    .lock()
                    .unwrap()
                    .entry(ctx.node_id.clone())
                    .or_default() += 1;
                this.received
                    .lock()
                    .unwrap()
                    .insert(ctx.node_id.clone(), inputs);
                let behavior = this
                    .behaviors
                    .get(&ctx.node_id)
                    .cloned()
                    .unwrap_or(Behavior {
                        delay: Duration::ZERO,
                        mode: Mode::Ok,
                    });
                tokio::time::sleep(behavior.delay).await;

                let result = match behavior.mode {
                    Mode::Fail(message) => Err(NodeError::Failed(message.to_owned())),
                    Mode::Panic => panic!("runner exploded"),
                    mode => build_outputs(&ctx, &mode).await,
                };
                this.spans
                    .lock()
                    .unwrap()
                    .insert(ctx.node_id.clone(), (started, Instant::now()));
                result
            }
            .boxed()
        }
    }

    /// Writes dummy files for every catalog output socket and returns matching artifacts.
    async fn build_outputs(ctx: &NodeContext, mode: &Mode) -> Result<NodeOutputs, NodeError> {
        let mut outputs = NodeOutputs::new();
        for socket in &spec(ctx.kind).outputs {
            let scalar = socket.data_type.scalar;
            let mut paths = Vec::new();
            if socket.data_type.list {
                let path = list_output_path(&ctx.out_dir, &socket.id, scalar, 0);
                tokio::fs::create_dir_all(path.parent().unwrap())
                    .await
                    .unwrap();
                tokio::fs::write(&path, b"x").await.unwrap();
                paths.push(path);
            } else {
                let path = single_output_path(&ctx.out_dir, &socket.id, scalar);
                tokio::fs::write(&path, b"x").await.unwrap();
                paths.push(path);
                if matches!(mode, Mode::TwoFiles(id) if *id == socket.id) {
                    paths.push(ctx.out_dir.join("extra.bin"));
                }
            }
            let data_type = match mode {
                Mode::WrongType(id) if *id == socket.id => {
                    DataType::single(if scalar == crate::types::ScalarType::Mp3 {
                        crate::types::ScalarType::Pdf
                    } else {
                        crate::types::ScalarType::Mp3
                    })
                }
                _ => socket.data_type,
            };
            if matches!(mode, Mode::OmitSocket(id) if *id == socket.id) {
                continue;
            }
            outputs.insert(socket.id.clone(), Artifact { data_type, paths });
        }
        Ok(outputs)
    }

    #[derive(Default)]
    struct RecordingObserver {
        updates: Mutex<Vec<NodeUpdate>>,
    }

    impl RecordingObserver {
        fn updates(&self) -> Vec<NodeUpdate> {
            self.updates.lock().unwrap().clone()
        }

        fn statuses_of(&self, node_id: &str) -> Vec<NodeStatus> {
            self.updates()
                .iter()
                .filter(|u| u.node_id == node_id)
                .map(|u| u.status)
                .collect()
        }

        fn update_of(&self, node_id: &str, status: NodeStatus) -> NodeUpdate {
            self.updates()
                .into_iter()
                .find(|u| u.node_id == node_id && u.status == status)
                .unwrap_or_else(|| panic!("no {status:?} update for {node_id}"))
        }
    }

    impl RunObserver for Arc<RecordingObserver> {
        fn node_changed(&self, update: NodeUpdate) -> BoxFuture<'static, ()> {
            self.updates.lock().unwrap().push(update);
            async {}.boxed()
        }
    }

    /// Temporary data dir removed on drop.
    struct TempData(PathBuf);

    impl TempData {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("dochestra-test-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempData {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ---------------------------------------------------------------- graph helpers

    fn node(id: &str, kind: NodeKind, is_output: bool) -> GraphNode {
        let params = match kind {
            NodeKind::Bundle => json!({"bundleId": "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f"}),
            other => spec(other).default_params.clone(),
        };
        GraphNode {
            id: id.to_owned(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            params,
            is_output,
        }
    }

    fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> GraphEdge {
        GraphEdge {
            id: id.to_owned(),
            source: s.to_owned(),
            source_handle: sh.to_owned(),
            target: t.to_owned(),
            target_handle: th.to_owned(),
        }
    }

    fn plan_of(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> ExecutionPlan {
        plan_run(&WorkflowGraph { nodes, edges }).expect("test graph must be valid")
    }

    /// bundle → pick_pdf → extract(output)
    fn chain() -> ExecutionPlan {
        plan_of(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p", NodeKind::PickPdf, false),
                node("x", NodeKind::Extract, true),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
            ],
        )
    }

    /// bundle → {p1 → x1(output)}, {p2 → x2(output)}
    fn two_branches() -> ExecutionPlan {
        plan_of(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p1", NodeKind::PickPdf, false),
                node("x1", NodeKind::Extract, true),
                node("p2", NodeKind::PickPdf, false),
                node("x2", NodeKind::Extract, true),
            ],
            vec![
                edge("e1", "b", "files", "p1", "files"),
                edge("e2", "p1", "pdf", "x1", "pdf"),
                edge("e3", "b", "files", "p2", "files"),
                edge("e4", "p2", "pdf", "x2", "pdf"),
            ],
        )
    }

    /// bundle → pick → extract → {summary(output), rag(output)}
    fn fan_out() -> ExecutionPlan {
        plan_of(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p", NodeKind::PickPdf, false),
                node("x", NodeKind::Extract, false),
                node("s", NodeKind::Summary, true),
                node("r", NodeKind::Rag, true),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
                edge("e3", "x", "md", "s", "md"),
                edge("e4", "x", "md", "r", "md"),
            ],
        )
    }

    /// b → p1 → x1 → s1(output)   and   b → p2 → x2(output)
    fn failing_graph() -> ExecutionPlan {
        plan_of(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p1", NodeKind::PickPdf, false),
                node("x1", NodeKind::Extract, false),
                node("s1", NodeKind::Summary, true),
                node("p2", NodeKind::PickPdf, false),
                node("x2", NodeKind::Extract, true),
            ],
            vec![
                edge("e1", "b", "files", "p1", "files"),
                edge("e2", "p1", "pdf", "x1", "pdf"),
                edge("e3", "x1", "md", "s1", "md"),
                edge("e4", "b", "files", "p2", "files"),
                edge("e5", "p2", "pdf", "x2", "pdf"),
            ],
        )
    }

    async fn run(
        plan: &ExecutionPlan,
        runner: FakeRunner,
    ) -> (
        RunOutcome,
        Arc<FakeRunner>,
        Arc<RecordingObserver>,
        TempData,
    ) {
        let data = TempData::new();
        let runner = Arc::new(runner);
        let observer = Arc::new(RecordingObserver::default());
        let outcome = execute(
            plan,
            Uuid::new_v4(),
            &data.0,
            Arc::new(Arc::clone(&runner)),
            Arc::new(Arc::clone(&observer)),
        )
        .await;
        (outcome, runner, observer, data)
    }

    // ---------------------------------------------------------------- tests

    #[tokio::test]
    async fn linear_chain_runs_in_order_and_succeeds() {
        let (outcome, runner, observer, _data) = run(&chain(), FakeRunner::default()).await;
        assert_eq!(outcome.status, RunStatus::Succeeded);
        assert!(outcome.nodes.values().all(|s| *s == NodeStatus::Succeeded));
        assert_eq!(outcome.nodes.len(), 3);

        // Each node starts only after its upstream finished.
        assert!(runner.span("b").1 <= runner.span("p").0);
        assert!(runner.span("p").1 <= runner.span("x").0);

        let succeeded: Vec<String> = observer
            .updates()
            .into_iter()
            .filter(|u| u.status == NodeStatus::Succeeded)
            .map(|u| u.node_id)
            .collect();
        assert_eq!(succeeded, ["b", "p", "x"]);
        // Outputs are files under {data}/runs/{run}/{node}/.
        let outputs = observer
            .update_of("x", NodeStatus::Succeeded)
            .outputs
            .unwrap();
        assert_eq!(outputs.len(), 4); // md, json, tables, images
        assert!(outputs["md"].paths[0].exists());
    }

    #[tokio::test]
    async fn independent_branches_run_in_parallel() {
        let runner = FakeRunner::default()
            .with("p1", 200, Mode::Ok)
            .with("p2", 200, Mode::Ok);
        let started = Instant::now();
        let (outcome, runner, _observer, _data) = run(&two_branches(), runner).await;
        let elapsed = started.elapsed();

        assert_eq!(outcome.status, RunStatus::Succeeded);
        let (s1, e1) = runner.span("p1");
        let (s2, e2) = runner.span("p2");
        assert!(s1 < e2 && s2 < e1, "branch spans must overlap");
        assert!(elapsed < Duration::from_millis(350), "took {elapsed:?}");
    }

    #[tokio::test]
    async fn fan_out_reuses_a_single_run_of_the_shared_node() {
        let (outcome, runner, observer, _data) = run(&fan_out(), FakeRunner::default()).await;
        assert_eq!(outcome.status, RunStatus::Succeeded);
        assert_eq!(runner.calls("x"), 1);
        assert_eq!(
            observer.statuses_of("x"),
            [NodeStatus::Running, NodeStatus::Succeeded]
        );

        let md = observer
            .update_of("x", NodeStatus::Succeeded)
            .outputs
            .unwrap()["md"]
            .clone();
        assert_eq!(runner.inputs_of("s")["md"], md);
        assert_eq!(runner.inputs_of("r")["md"], md);
    }

    #[tokio::test]
    async fn failure_skips_downstream_but_independent_branch_completes() {
        let runner = FakeRunner::default().with("p1", 0, Mode::Fail("boom"));
        let (outcome, runner, observer, _data) = run(&failing_graph(), runner).await;

        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(outcome.nodes["p1"], NodeStatus::Failed);
        assert_eq!(outcome.nodes["x1"], NodeStatus::Skipped);
        assert_eq!(outcome.nodes["s1"], NodeStatus::Skipped);
        assert_eq!(outcome.nodes["p2"], NodeStatus::Succeeded);
        assert_eq!(outcome.nodes["x2"], NodeStatus::Succeeded);

        // Skipped nodes are reported once, never Running, and never invoked.
        for id in ["x1", "s1"] {
            assert_eq!(observer.statuses_of(id), [NodeStatus::Skipped]);
            assert_eq!(runner.calls(id), 0);
        }
        let failed = observer.update_of("p1", NodeStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("boom"));
        assert!(failed.outputs.is_none());
    }

    #[tokio::test]
    async fn skipped_nodes_are_reported_in_plan_order() {
        let runner = FakeRunner::default().with("p1", 0, Mode::Fail("boom"));
        let (_, _, observer, _data) = run(&failing_graph(), runner).await;
        let skipped: Vec<String> = observer
            .updates()
            .into_iter()
            .filter(|u| u.status == NodeStatus::Skipped)
            .map(|u| u.node_id)
            .collect();
        assert_eq!(skipped, ["x1", "s1"]);
    }

    #[tokio::test]
    async fn runner_panic_fails_only_that_node() {
        let runner = FakeRunner::default().with("x1", 0, Mode::Panic);
        let (outcome, _runner, observer, _data) = run(&failing_graph(), runner).await;
        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(outcome.nodes["x1"], NodeStatus::Failed);
        assert_eq!(outcome.nodes["s1"], NodeStatus::Skipped);
        assert_eq!(outcome.nodes["x2"], NodeStatus::Succeeded);
        let error = observer.update_of("x1", NodeStatus::Failed).error.unwrap();
        assert!(
            error.contains("패닉") && error.contains("runner exploded"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn output_contract_violations_fail_the_node() {
        let cases = [
            (Mode::OmitSocket("md"), "출력 소켓 md 누락"),
            (Mode::WrongType("md"), "출력 소켓 md 타입 불일치"),
            (Mode::TwoFiles("md"), "출력 소켓 md"),
        ];
        for (mode, expected) in cases {
            let runner = FakeRunner::default().with("x", 0, mode);
            let (outcome, _runner, observer, _data) = run(&chain(), runner).await;
            assert_eq!(outcome.status, RunStatus::Failed);
            assert_eq!(outcome.nodes["x"], NodeStatus::Failed);
            let error = observer.update_of("x", NodeStatus::Failed).error.unwrap();
            assert!(
                error.contains(expected),
                "{error} should contain {expected}"
            );
            // The invalid outputs are not reported as a success.
            assert!(!observer.statuses_of("x").contains(&NodeStatus::Succeeded));
        }
    }

    #[tokio::test]
    async fn inputs_are_keyed_by_consumer_socket_id() {
        let (_, runner, observer, _data) = run(&chain(), FakeRunner::default()).await;
        let inputs = runner.inputs_of("x");
        assert_eq!(inputs.keys().collect::<Vec<_>>(), ["pdf"]);
        let produced = observer
            .update_of("p", NodeStatus::Succeeded)
            .outputs
            .unwrap();
        assert_eq!(inputs["pdf"], produced["pdf"]);
        assert!(runner.inputs_of("b").is_empty());
    }

    #[tokio::test]
    async fn observer_sees_running_then_exactly_one_terminal_per_node() {
        for runner in [
            FakeRunner::default(),
            FakeRunner::default().with("p1", 0, Mode::Fail("x")),
        ] {
            let plan = failing_graph();
            let (outcome, _runner, observer, _data) = run(&plan, runner).await;
            for node in &plan.nodes {
                let statuses = observer.statuses_of(&node.id);
                let terminal: Vec<_> = statuses
                    .iter()
                    .filter(|s| {
                        matches!(
                            s,
                            NodeStatus::Succeeded | NodeStatus::Failed | NodeStatus::Skipped
                        )
                    })
                    .collect();
                assert_eq!(terminal.len(), 1, "{}: {statuses:?}", node.id);
                assert_eq!(statuses.last(), terminal.first().copied(), "{}", node.id);
                assert!(!statuses.contains(&NodeStatus::Pending));
                match outcome.nodes[&node.id] {
                    NodeStatus::Skipped => assert_eq!(statuses.len(), 1),
                    _ => assert_eq!(statuses[0], NodeStatus::Running, "{}", node.id),
                }
            }
        }
    }

    #[tokio::test]
    async fn node_output_directories_are_created_under_the_run() {
        let data = TempData::new();
        let run_id = Uuid::new_v4();
        let observer = Arc::new(RecordingObserver::default());
        let runner = Arc::new(FakeRunner::default());
        execute(
            &chain(),
            run_id,
            &data.0,
            Arc::new(Arc::clone(&runner)),
            Arc::new(Arc::clone(&observer)),
        )
        .await;
        for id in ["b", "p", "x"] {
            assert!(
                data.0
                    .join("runs")
                    .join(run_id.to_string())
                    .join(id)
                    .is_dir()
            );
        }
    }

    #[tokio::test]
    async fn unsafe_node_id_fails_without_running() {
        let mut graph_node = node("b", NodeKind::Bundle, true);
        graph_node.id = "../escape".to_owned();
        let plan = plan_of(vec![graph_node], vec![]);
        let (outcome, runner, observer, _data) = run(&plan, FakeRunner::default()).await;
        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(runner.calls("../escape"), 0);
        assert_eq!(observer.statuses_of("../escape"), [NodeStatus::Failed]);
    }

    #[tokio::test]
    async fn empty_plan_succeeds() {
        let plan = ExecutionPlan {
            nodes: vec![],
            outputs: vec![],
        };
        let (outcome, _, observer, _data) = run(&plan, FakeRunner::default()).await;
        assert_eq!(outcome.status, RunStatus::Succeeded);
        assert!(outcome.nodes.is_empty());
        assert!(observer.updates().is_empty());
    }
}
