use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BundleFile {
    pub id: Uuid,
    pub original_name: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bundle {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub files: Vec<BundleFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BundleSummary {
    pub id: Uuid,
    pub name: String,
    #[ts(type = "number")]
    pub file_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateBundleRequest {
    pub name: String,
}
