//! Run endpoints: start, list, detail and SSE (CONTEXT §4.1.3, §4.1.4).

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderName, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream;
use tokio::sync::broadcast::{self, error::RecvError};
use uuid::Uuid;

use super::extract::ApiPath;
use crate::db;
use crate::error::ApiError;
use crate::executor::events::EventHub;
use crate::executor::execute;
use crate::executor::observer::DbObserver;
use crate::graph::planner::{ExecutionPlan, plan_run};
use crate::state::AppState;
use crate::types::{NodeStatus, RunCreated, RunDetail, RunEvent, RunStatus, RunSummary};

const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// How long the SSE handler waits for a just-created run's channel to appear.
const SUBSCRIBE_RETRIES: u32 = 20;
const SUBSCRIBE_RETRY_DELAY: Duration = Duration::from_millis(50);

fn run_not_found(id: Uuid) -> ApiError {
    ApiError::NotFound(format!("run {id} not found"))
}

/// `POST /api/workflows/{id}/runs`: validate, persist, execute in the background.
pub async fn create(
    State(state): State<AppState>,
    ApiPath(workflow_id): ApiPath<Uuid>,
) -> Result<(StatusCode, Json<RunCreated>), ApiError> {
    let workflow = db::workflows::get(&state.db, workflow_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("workflow {workflow_id} not found")))?;
    let plan = plan_run(&workflow.graph).map_err(|issues| ApiError::Validation {
        message: "실행 전 검증 실패".to_owned(),
        issues,
    })?;

    let run_id = db::runs::create_run(&state.db, workflow.id, &workflow.graph, &plan).await?;
    state.events.create(run_id);
    tokio::spawn(run_in_background(state, run_id, plan));

    Ok((StatusCode::ACCEPTED, Json(RunCreated { run_id })))
}

/// Executes the plan, then records the final status (DB first) and announces it.
async fn run_in_background(state: AppState, run_id: Uuid, plan: ExecutionPlan) {
    let observer = DbObserver {
        db: state.db.clone(),
        events: state.events.clone(),
        run_id,
        kinds: plan.nodes.iter().map(|n| (n.id.clone(), n.kind)).collect(),
    };
    let outcome = execute(
        &plan,
        run_id,
        &state.config.data_dir,
        state.runner.clone(),
        std::sync::Arc::new(observer),
    )
    .await;

    if let Err(err) = db::runs::finish_run(&state.db, run_id, outcome.status).await {
        tracing::error!(%run_id, error = ?err, "failed to persist run result");
    }
    tracing::info!(%run_id, status = ?outcome.status, "run finished");
    state.events.send(
        run_id,
        RunEvent::RunFinished {
            status: outcome.status,
        },
    );
    state.events.remove(run_id);
}

/// `GET /api/workflows/{id}/runs`
pub async fn list(
    State(state): State<AppState>,
    ApiPath(workflow_id): ApiPath<Uuid>,
) -> Result<Json<Vec<RunSummary>>, ApiError> {
    if db::workflows::get(&state.db, workflow_id).await?.is_none() {
        return Err(ApiError::NotFound(format!(
            "workflow {workflow_id} not found"
        )));
    }
    Ok(Json(
        db::runs::list_for_workflow(&state.db, workflow_id).await?,
    ))
}

/// `GET /api/runs/{runId}`
pub async fn detail(
    State(state): State<AppState>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<Json<RunDetail>, ApiError> {
    db::runs::get_detail(&state.db, run_id)
        .await?
        .map(Json)
        .ok_or_else(|| run_not_found(run_id))
}

struct SseState {
    db: sqlx::PgPool,
    run_id: Uuid,
    rx: Option<broadcast::Receiver<RunEvent>>,
    /// Events already decided but not yet delivered.
    pending: VecDeque<RunEvent>,
    /// Set once `RunFinished` has been queued: the stream ends after delivering it.
    finishing: bool,
    /// Last status told to this client per node (initialized from every snapshot), used to
    /// keep each node's statuses monotonic on this connection.
    sent: HashMap<String, NodeStatus>,
}

fn rank(status: NodeStatus) -> u8 {
    match status {
        NodeStatus::Pending => 0,
        NodeStatus::Running => 1,
        NodeStatus::Succeeded | NodeStatus::Failed | NodeStatus::Skipped => 2,
    }
}

fn is_terminal(status: NodeStatus) -> bool {
    rank(status) == 2
}

/// Whether a `node_status` broadcast should reach a client that was last told `last`.
///
/// Statuses only move forward (pending < running < terminal). A broadcast can be stale
/// because the snapshot was read after it had already been persisted; such events, repeats and
/// terminal-to-different-terminal changes are dropped.
pub fn should_forward(last: Option<NodeStatus>, next: NodeStatus) -> bool {
    last.is_none_or(|prev| rank(next) > rank(prev))
}

impl SseState {
    /// Queues an event for delivery, dropping stale `node_status` events (see
    /// [`should_forward`]) and re-basing the per-node state on every snapshot.
    fn queue(&mut self, event: RunEvent) {
        match &event {
            RunEvent::Snapshot { run } => {
                self.sent = run
                    .nodes
                    .iter()
                    .map(|n| (n.node_id.clone(), n.status))
                    .collect();
            }
            RunEvent::NodeStatus {
                node_id, status, ..
            } => {
                let last = self.sent.get(node_id).copied();
                if !should_forward(last, *status) {
                    if let Some(prev) = last
                        && is_terminal(prev)
                        && is_terminal(*status)
                        && prev != *status
                    {
                        tracing::warn!(
                            run_id = %self.run_id, %node_id, ?prev, next = ?status,
                            "dropping conflicting terminal status"
                        );
                    }
                    return;
                }
                self.sent.insert(node_id.clone(), *status);
            }
            RunEvent::RunFinished { .. } => self.finishing = true,
        }
        self.pending.push_back(event);
    }

    /// Re-reads the run from the DB; queues a fresh snapshot (when `snapshot`) and, if the run
    /// is already over, its `RunFinished`. Returns whether the run is over.
    async fn resync(&mut self, snapshot: bool) -> bool {
        match db::runs::get_detail(&self.db, self.run_id).await {
            Ok(Some(detail)) => {
                let status = detail.status;
                if snapshot {
                    self.queue(RunEvent::Snapshot { run: detail });
                }
                if status != RunStatus::Running {
                    self.queue(RunEvent::RunFinished { status });
                    return true;
                }
                false
            }
            Ok(None) => {
                // Run deleted (e.g. its workflow was removed): nothing more to say.
                self.finishing = true;
                true
            }
            Err(err) => {
                tracing::error!(run_id = %self.run_id, error = ?err, "SSE resync failed");
                false
            }
        }
    }
}

fn to_sse_event(event: &RunEvent) -> Event {
    Event::default()
        .json_data(event)
        .unwrap_or_else(|_| Event::default().comment("serialization error"))
}

/// Subscribes to the run's channel, retrying briefly: the channel is created right after the
/// run row is inserted, so a client that learned the id elsewhere may be a few ms early.
async fn subscribe_with_retry(
    events: &EventHub,
    db: &sqlx::PgPool,
    run_id: Uuid,
) -> Option<broadcast::Receiver<RunEvent>> {
    for _ in 0..SUBSCRIBE_RETRIES {
        tokio::time::sleep(SUBSCRIBE_RETRY_DELAY).await;
        if let Some(rx) = events.subscribe(run_id) {
            return Some(rx);
        }
        // Finished meanwhile? Then there will never be a channel.
        if let Ok(Some(detail)) = db::runs::get_detail(db, run_id).await
            && detail.status != RunStatus::Running
        {
            return None;
        }
    }
    None
}

/// `GET /api/runs/{runId}/events`: snapshot, then live events until `run_finished`.
pub async fn events(
    State(state): State<AppState>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    // Subscribe before reading the snapshot so no event can fall in between; the observer
    // persists before it publishes, so snapshot + events never miss a transition.
    let mut rx = state.events.subscribe(run_id);
    let detail = db::runs::get_detail(&state.db, run_id)
        .await?
        .ok_or_else(|| run_not_found(run_id))?;

    let mut sse = SseState {
        db: state.db.clone(),
        run_id,
        rx: None,
        pending: VecDeque::new(),
        finishing: false,
        sent: HashMap::new(),
    };
    let status = detail.status;
    sse.queue(RunEvent::Snapshot { run: detail });

    if status != RunStatus::Running {
        sse.queue(RunEvent::RunFinished { status });
    } else {
        if rx.is_none() {
            rx = subscribe_with_retry(&state.events, &state.db, run_id).await;
        }
        match rx {
            Some(rx) => sse.rx = Some(rx),
            None => {
                // No live channel: it either finished just now (resync emits the final state)
                // or never started; in the latter case the stream simply ends after the
                // snapshot and the client's EventSource reconnects.
                sse.pending.clear();
                if !sse.resync(true).await {
                    sse.finishing = true;
                }
            }
        }
    }

    let body = stream::unfold(sse, |mut st| async move {
        loop {
            if let Some(event) = st.pending.pop_front() {
                return Some((Ok::<_, Infallible>(to_sse_event(&event)), st));
            }
            if st.finishing {
                return None;
            }
            let rx = st.rx.as_mut()?;
            match rx.recv().await {
                Ok(event) => st.queue(event),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(run_id = %st.run_id, skipped, "SSE subscriber lagged; resending snapshot");
                    // A fresh snapshot (and the final event if the run is over); live events
                    // continue afterwards.
                    st.resync(true).await;
                }
                Err(RecvError::Closed) => {
                    if !st.resync(false).await {
                        st.finishing = true;
                    }
                }
            }
        }
    });

    let sse = Sse::new(body).keep_alive(KeepAlive::new().interval(KEEP_ALIVE));
    Ok(([(HeaderName::from_static("x-accel-buffering"), "no")], sse).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [NodeStatus; 5] = [
        NodeStatus::Pending,
        NodeStatus::Running,
        NodeStatus::Succeeded,
        NodeStatus::Failed,
        NodeStatus::Skipped,
    ];

    #[test]
    fn unknown_nodes_are_forwarded() {
        for next in ALL {
            assert!(should_forward(None, next));
        }
    }

    #[test]
    fn only_forward_progress_is_forwarded() {
        use NodeStatus::{Failed, Pending, Running, Skipped, Succeeded};
        // Advancing.
        assert!(should_forward(Some(Pending), Running));
        assert!(should_forward(Some(Pending), Succeeded));
        assert!(should_forward(Some(Pending), Skipped));
        assert!(should_forward(Some(Running), Succeeded));
        assert!(should_forward(Some(Running), Failed));
        // Repeats.
        for status in ALL {
            assert!(!should_forward(Some(status), status), "{status:?}");
        }
        // Regressions.
        assert!(!should_forward(Some(Running), Pending));
        assert!(!should_forward(Some(Succeeded), Running));
        assert!(!should_forward(Some(Failed), Running));
        assert!(!should_forward(Some(Skipped), Pending));
        // Terminal -> different terminal.
        assert!(!should_forward(Some(Succeeded), Failed));
        assert!(!should_forward(Some(Failed), Succeeded));
        assert!(!should_forward(Some(Skipped), Succeeded));
    }
}
