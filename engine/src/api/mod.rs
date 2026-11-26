//! HTTP routing.

pub mod artifacts;
pub mod bundles;
pub mod extract;
pub mod health;
pub mod nodes;
pub mod runs;
pub mod workflows;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let upload_limit = state.config.upload_limit_bytes;
    Router::new()
        .route("/api/health", get(health::health))
        .route("/api/nodes", get(nodes::list_nodes))
        .route(
            "/api/workflows",
            get(workflows::list).post(workflows::create),
        )
        .route(
            "/api/workflows/{id}",
            get(workflows::get)
                .put(workflows::update)
                .delete(workflows::delete),
        )
        .route("/api/workflows/{id}/validate", post(workflows::validate))
        .route(
            "/api/workflows/{id}/runs",
            get(runs::list).post(runs::create),
        )
        .route(
            "/api/runs/{run_id}/artifacts/{node_id}/{socket_id}",
            get(artifacts::download),
        )
        .route("/api/bundles", get(bundles::list).post(bundles::create))
        .route(
            "/api/bundles/{id}",
            get(bundles::get).delete(bundles::delete),
        )
        .route(
            "/api/bundles/{id}/files",
            post(bundles::upload_files).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route(
            "/api/bundles/{id}/files/{file_id}",
            delete(bundles::delete_file),
        )
        .route("/api/runs/{run_id}", get(runs::detail))
        .route("/api/runs/{run_id}/events", get(runs::events))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
