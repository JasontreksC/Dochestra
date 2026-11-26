//! Node execution contract.

use std::collections::HashMap;
use std::path::PathBuf;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{DataType, NodeKind};

/// Data produced by one output socket. Persisted as `node_runs.outputs[socket]` (CONTEXT §3.5):
/// single types have exactly one path, `LIST` types any number (possibly zero).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub data_type: DataType,
    pub paths: Vec<PathBuf>,
}

/// Input artifacts keyed by the consumer's input socket id.
pub type NodeInputs = HashMap<String, Artifact>;
/// Output artifacts keyed by the producer's output socket id.
pub type NodeOutputs = HashMap<String, Artifact>;

#[derive(Debug, Clone)]
pub struct NodeContext {
    pub run_id: Uuid,
    pub node_id: String,
    pub kind: NodeKind,
    pub params: serde_json::Value,
    /// `{DATA_DIR}/runs/{run_id}/{node_id}` (already created).
    pub out_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// A user-presentable failure.
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Executes one node. Implementations must write outputs under `ctx.out_dir` (using
/// `crate::storage` helpers) and return an artifact for every output socket of the node kind.
pub trait NodeRunner: Send + Sync + 'static {
    fn run(
        &self,
        ctx: NodeContext,
        inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>>;
}

/// `snake_case` name of a node kind (as used in JSON and the `node_runs.kind` column).
pub fn kind_name(kind: NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{kind:?}"))
}

/// Placeholder runner until the real node implementations land (M3): fails every node.
pub struct UnimplementedRunner;

impl NodeRunner for UnimplementedRunner {
    fn run(
        &self,
        ctx: NodeContext,
        _inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
        let message = format!("노드 '{}' 는 아직 구현되지 않았습니다", kind_name(ctx.kind));
        Box::pin(async move { Err(NodeError::Failed(message)) })
    }
}
