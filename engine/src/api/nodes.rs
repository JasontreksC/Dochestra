//! `GET /api/nodes`.

use axum::Json;

use crate::catalog::catalog;
use crate::types::NodeSpec;

pub async fn list_nodes() -> Json<&'static [NodeSpec]> {
    Json(catalog())
}
