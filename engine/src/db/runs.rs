//! `runs` / `node_runs` queries and output conversions.

use anyhow::{Context, anyhow};
use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

use crate::catalog::spec;
use crate::executor::runner::{NodeOutputs, kind_name};
use crate::graph::planner::ExecutionPlan;
use crate::types::{
    ArtifactFileRef, NodeKind, NodeOutput, NodeRunState, NodeStatus, RunDetail, RunStatus,
    RunSummary, WorkflowGraph,
};

const RESTART_ERROR: &str = "엔진 재시작으로 중단됨";
const LIST_LIMIT: i64 = 50;

pub fn run_status_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

pub fn node_status_str(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Running => "running",
        NodeStatus::Succeeded => "succeeded",
        NodeStatus::Failed => "failed",
        NodeStatus::Skipped => "skipped",
    }
}

fn parse_run_status(raw: &str) -> anyhow::Result<RunStatus> {
    match raw {
        "running" => Ok(RunStatus::Running),
        "succeeded" => Ok(RunStatus::Succeeded),
        "failed" => Ok(RunStatus::Failed),
        other => Err(anyhow!("unknown run status {other:?} in database")),
    }
}

fn parse_node_status(raw: &str) -> anyhow::Result<NodeStatus> {
    match raw {
        "pending" => Ok(NodeStatus::Pending),
        "running" => Ok(NodeStatus::Running),
        "succeeded" => Ok(NodeStatus::Succeeded),
        "failed" => Ok(NodeStatus::Failed),
        "skipped" => Ok(NodeStatus::Skipped),
        other => Err(anyhow!("unknown node status {other:?} in database")),
    }
}

fn parse_kind(raw: &str) -> anyhow::Result<NodeKind> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned()))
        .with_context(|| format!("unknown node kind {raw:?} in database"))
}

/// Converts stored/produced outputs into the API shape: sockets in catalog order, one
/// [`ArtifactFileRef`] per file with a download URL (CONTEXT §3.5, §4.1.3).
pub fn to_node_outputs(
    run_id: Uuid,
    node_id: &str,
    kind: NodeKind,
    outputs: &NodeOutputs,
) -> Vec<NodeOutput> {
    spec(kind)
        .outputs
        .iter()
        .filter_map(|socket| {
            let artifact = outputs.get(&socket.id)?;
            let files = artifact
                .paths
                .iter()
                .enumerate()
                .map(|(i, path)| ArtifactFileRef {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    file_name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    url: format!(
                        "/api/runs/{run_id}/artifacts/{node_id}/{}?index={i}",
                        socket.id
                    ),
                })
                .collect();
            Some(NodeOutput {
                socket_id: socket.id.clone(),
                data_type: artifact.data_type,
                files,
            })
        })
        .collect()
}

/// Inserts the run (`running`) and one `pending` node row per planned node, atomically.
pub async fn create_run(
    db: &PgPool,
    workflow_id: Uuid,
    graph_snapshot: &WorkflowGraph,
    plan: &ExecutionPlan,
) -> Result<Uuid, sqlx::Error> {
    let mut tx = db.begin().await?;
    let run_id = sqlx::query_scalar!(
        "INSERT INTO runs (workflow_id, status, graph_snapshot) VALUES ($1, 'running', $2) RETURNING id",
        workflow_id,
        Json(graph_snapshot) as Json<&WorkflowGraph>
    )
    .fetch_one(&mut *tx)
    .await?;

    for (seq, node) in plan.nodes.iter().enumerate() {
        let seq = i32::try_from(seq).unwrap_or(i32::MAX);
        sqlx::query!(
            "INSERT INTO node_runs (run_id, node_id, kind, status, seq) VALUES ($1, $2, $3, 'pending', $4)",
            run_id,
            node.id,
            kind_name(node.kind),
            seq
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(run_id)
}

/// Records a node state change: `running` sets `started_at`, terminal states set `finished_at`.
pub async fn update_node(
    db: &PgPool,
    run_id: Uuid,
    node_id: &str,
    status: NodeStatus,
    error: Option<&str>,
    outputs: Option<&NodeOutputs>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE node_runs
           SET status = $3,
               error = $4,
               outputs = COALESCE($5, outputs),
               started_at = CASE WHEN $3 = 'running' THEN now() ELSE started_at END,
               finished_at = CASE WHEN $3 IN ('succeeded', 'failed', 'skipped') THEN now() ELSE finished_at END
           WHERE run_id = $1 AND node_id = $2"#,
        run_id,
        node_id,
        node_status_str(status),
        error,
        outputs.map(Json) as Option<Json<&NodeOutputs>>
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn finish_run(db: &PgPool, run_id: Uuid, status: RunStatus) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE runs SET status = $2, finished_at = now() WHERE id = $1",
        run_id,
        run_status_str(status)
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Full run state with nodes ordered by plan sequence.
pub async fn get_detail(db: &PgPool, run_id: Uuid) -> anyhow::Result<Option<RunDetail>> {
    let Some(run) = sqlx::query!(
        "SELECT id, workflow_id, status, started_at, finished_at FROM runs WHERE id = $1",
        run_id
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };

    let rows = sqlx::query!(
        r#"SELECT node_id, kind, status, error,
                  outputs AS "outputs?: Json<NodeOutputs>",
                  started_at, finished_at
           FROM node_runs WHERE run_id = $1 ORDER BY seq, node_id"#,
        run_id
    )
    .fetch_all(db)
    .await?;

    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = parse_kind(&row.kind)?;
        let outputs = row
            .outputs
            .map(|Json(o)| to_node_outputs(run_id, &row.node_id, kind, &o))
            .unwrap_or_default();
        nodes.push(NodeRunState {
            node_id: row.node_id,
            kind,
            status: parse_node_status(&row.status)?,
            error: row.error,
            outputs,
            started_at: row.started_at,
            finished_at: row.finished_at,
        });
    }

    Ok(Some(RunDetail {
        id: run.id,
        workflow_id: run.workflow_id,
        status: parse_run_status(&run.status)?,
        started_at: run.started_at,
        finished_at: run.finished_at,
        nodes,
    }))
}

/// Stored outputs of one node, or `None` if the run/node does not exist or has no outputs yet.
pub async fn get_node_outputs(
    db: &PgPool,
    run_id: Uuid,
    node_id: &str,
) -> Result<Option<NodeOutputs>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT outputs AS "outputs?: Json<NodeOutputs>" FROM node_runs WHERE run_id = $1 AND node_id = $2"#,
        run_id,
        node_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.outputs).map(|Json(o)| o))
}

/// Latest runs of a workflow (newest first, at most 50).
pub async fn list_for_workflow(db: &PgPool, workflow_id: Uuid) -> anyhow::Result<Vec<RunSummary>> {
    let rows = sqlx::query!(
        "SELECT id, status, started_at, finished_at FROM runs
         WHERE workflow_id = $1 ORDER BY started_at DESC, id LIMIT $2",
        workflow_id,
        LIST_LIMIT
    )
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(RunSummary {
                id: r.id,
                status: parse_run_status(&r.status)?,
                started_at: r.started_at,
                finished_at: r.finished_at,
            })
        })
        .collect()
}

/// Startup cleanup (CONTEXT §5): runs still `running` become `failed`; their `running` nodes
/// become `failed` ("엔진 재시작으로 중단됨") and `pending` nodes `skipped`. Returns the number
/// of runs cleaned up.
pub async fn fail_orphaned_runs(db: &PgPool) -> Result<u64, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query!(
        "UPDATE node_runs SET status = 'failed', error = $1, finished_at = now()
         WHERE status = 'running' AND run_id IN (SELECT id FROM runs WHERE status = 'running')",
        RESTART_ERROR
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE node_runs SET status = 'skipped', finished_at = now()
         WHERE status = 'pending' AND run_id IN (SELECT id FROM runs WHERE status = 'running')"
    )
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query!(
        "UPDATE runs SET status = 'failed', finished_at = now() WHERE status = 'running'"
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::executor::runner::Artifact;
    use crate::types::{DataType, ScalarType};

    fn artifact(data_type: DataType, paths: &[&str]) -> Artifact {
        Artifact {
            data_type,
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn outputs_follow_catalog_order_and_build_urls() {
        let run = Uuid::parse_str("3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f").unwrap();
        let mut outputs = NodeOutputs::new();
        // Insert out of catalog order on purpose.
        outputs.insert(
            "images".into(),
            artifact(
                DataType::list_of(ScalarType::Jpg),
                &[
                    "/data/runs/r/n1/images/0000.jpg",
                    "/data/runs/r/n1/images/0001.jpg",
                ],
            ),
        );
        outputs.insert(
            "md".into(),
            artifact(DataType::single(ScalarType::Md), &["/data/runs/r/n1/md.md"]),
        );
        outputs.insert(
            "tables".into(),
            artifact(DataType::list_of(ScalarType::Csv), &[]),
        );
        outputs.insert(
            "json".into(),
            artifact(
                DataType::single(ScalarType::Json),
                &["/data/runs/r/n1/json.json"],
            ),
        );
        outputs.insert(
            "bogus".into(),
            artifact(DataType::single(ScalarType::Md), &["/x/y.md"]),
        );

        let out = to_node_outputs(run, "n1", NodeKind::Extract, &outputs);
        let sockets: Vec<&str> = out.iter().map(|o| o.socket_id.as_str()).collect();
        assert_eq!(sockets, ["md", "json", "tables", "images"]);

        assert_eq!(out[0].files.len(), 1);
        assert_eq!(out[0].files[0].file_name, "md.md");
        assert_eq!(
            out[0].files[0].url,
            "/api/runs/3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f/artifacts/n1/md?index=0"
        );
        assert!(out[2].files.is_empty());
        assert_eq!(out[3].files.len(), 2);
        assert_eq!(out[3].files[1].index, 1);
        assert_eq!(out[3].files[1].file_name, "0001.jpg");
        assert_eq!(
            out[3].files[1].url,
            "/api/runs/3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f/artifacts/n1/images?index=1"
        );
        assert_eq!(out[3].data_type, DataType::list_of(ScalarType::Jpg));
    }

    #[test]
    fn missing_sockets_are_skipped() {
        let out = to_node_outputs(Uuid::nil(), "n", NodeKind::Extract, &NodeOutputs::new());
        assert!(out.is_empty());
    }

    #[test]
    fn status_strings_round_trip() {
        for s in [RunStatus::Running, RunStatus::Succeeded, RunStatus::Failed] {
            assert_eq!(parse_run_status(run_status_str(s)).unwrap(), s);
        }
        for s in [
            NodeStatus::Pending,
            NodeStatus::Running,
            NodeStatus::Succeeded,
            NodeStatus::Failed,
            NodeStatus::Skipped,
        ] {
            assert_eq!(parse_node_status(node_status_str(s)).unwrap(), s);
        }
        assert!(parse_run_status("nope").is_err());
        assert_eq!(parse_kind("pick_pdf").unwrap(), NodeKind::PickPdf);
        assert!(parse_kind("teleport").is_err());
    }
}
