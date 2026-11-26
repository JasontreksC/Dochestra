//! Shared application state.

use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::executor::events::EventHub;
use crate::executor::runner::NodeRunner;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    /// Per-run event channels feeding SSE subscribers.
    pub events: EventHub,
    /// Executes workflow nodes (injectable for tests).
    pub runner: Arc<dyn NodeRunner>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: Config,
        http: reqwest::Client,
        runner: Arc<dyn NodeRunner>,
    ) -> Self {
        Self {
            db,
            config: Arc::new(config),
            http,
            events: EventHub::new(),
            runner,
        }
    }
}
