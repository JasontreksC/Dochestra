//! Dochestra engine: workflow API server and executor.

pub mod api;
pub mod catalog;
pub mod clients;
pub mod config;
pub mod db;
pub mod error;
pub mod executor;
pub mod graph;
pub mod nodes;
pub mod state;
pub mod storage;
pub mod types;

use axum::Router;

use crate::state::AppState;

/// Builds the full application router (used by `main` and integration tests).
pub fn build_app(state: AppState) -> Router {
    api::router(state)
}
