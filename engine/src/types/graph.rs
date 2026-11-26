use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::node::NodeKind;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    pub position: Position,
    #[serde(default = "empty_object")]
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
    #[serde(default)]
    pub is_output: bool,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub source_handle: String,
    pub target: String,
    pub target_handle: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
