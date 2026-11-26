//! Shared application state.

use std::sync::Arc;

use sqlx::PgPool;

use crate::clients::embedding::EmbeddingClient;
use crate::clients::llm::LlmClient;
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
    /// Question embedding for the RAG query API.
    pub embedder: EmbeddingClient,
    /// Answer generation for the RAG query API.
    pub llm: LlmClient,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: Config,
        http: reqwest::Client,
        runner: Arc<dyn NodeRunner>,
    ) -> Self {
        let embedder = EmbeddingClient::new(
            http.clone(),
            config.openrouter_base_url.clone(),
            config.openrouter_api_key.clone(),
            config.embedding_model.clone(),
        );
        let llm = LlmClient::new(
            http.clone(),
            config.openrouter_base_url.clone(),
            config.openrouter_api_key.clone(),
            config.llm_model.clone(),
            config.llm_effort.clone(),
        );
        Self {
            db,
            config: Arc::new(config),
            http,
            events: EventHub::new(),
            runner,
            embedder,
            llm,
        }
    }
}
