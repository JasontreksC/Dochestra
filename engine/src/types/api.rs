use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::data_type::DataType;
use super::graph::WorkflowGraph;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub graph: WorkflowGraph,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowSummary {
    pub id: Uuid,
    pub name: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateWorkflowRequest {
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateWorkflowRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub graph: Option<WorkflowGraph>,
}

/// Validation issues (CONTEXT §4.1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "code",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
#[ts(export, rename_all_fields = "camelCase")]
pub enum ValidationIssue {
    DuplicateId {
        id: String,
    },
    UnknownNode {
        edge_id: String,
        node_id: String,
    },
    UnknownSocket {
        edge_id: String,
        node_id: String,
        socket_id: String,
    },
    TypeMismatch {
        edge_id: String,
        expected: DataType,
        actual: DataType,
    },
    MultipleIncoming {
        node_id: String,
        socket_id: String,
    },
    CycleDetected {
        node_ids: Vec<String>,
    },
    NoOutputNode,
    UnconnectedInput {
        node_id: String,
        socket_id: String,
    },
    InvalidParams {
        node_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationReport {
    pub ok: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub issues: Option<Vec<ValidationIssue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RagQueryRequest {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub top_k: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RagSource {
    pub chunk_index: i32,
    pub content: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RagQueryResponse {
    pub answer: String,
    pub sources: Vec<RagSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HealthResponse {
    pub status: String,
}
