use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::data_type::DataType;
use super::node::NodeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArtifactFileRef {
    pub index: u32,
    pub file_name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeOutput {
    pub socket_id: String,
    pub data_type: DataType,
    pub files: Vec<ArtifactFileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeRunState {
    pub node_id: String,
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub error: Option<String>,
    pub outputs: Vec<NodeOutput>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunDetail {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub nodes: Vec<NodeRunState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunSummary {
    pub id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunCreated {
    pub run_id: Uuid,
}

/// SSE payload (`data:` of each message), CONTEXT §4.1.4.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, rename_all_fields = "camelCase")]
pub enum RunEvent {
    Snapshot {
        run: RunDetail,
    },
    NodeStatus {
        node_id: String,
        status: NodeStatus,
        error: Option<String>,
        outputs: Vec<NodeOutput>,
    },
    RunFinished {
        status: RunStatus,
    },
}
