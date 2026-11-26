use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::data_type::DataType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NodeKind {
    Bundle,
    PickPdf,
    PickJpg,
    JsonInput,
    Extract,
    Translate,
    Analyze,
    Summary,
    Slide,
    Voice,
    Fill,
    Rag,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SocketSpec {
    pub id: String,
    pub label: String,
    pub data_type: DataType,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeSpec {
    pub kind: NodeKind,
    pub label: String,
    pub description: String,
    pub inputs: Vec<SocketSpec>,
    pub outputs: Vec<SocketSpec>,
    #[ts(type = "unknown")]
    pub default_params: serde_json::Value,
}
