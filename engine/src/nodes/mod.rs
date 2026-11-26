//! Concrete node implementations and the production [`NodeRunner`].

pub mod analyze;
pub mod bundle;
pub mod extract;
pub mod json_input;
pub mod pick;
pub mod rag;
pub mod summary;
pub mod translate;

use std::path::Path;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::PgPool;

use crate::clients::embedding::EmbeddingClient;
use crate::clients::extractor::ExtractorClient;
use crate::clients::llm::LlmClient;
use crate::config::Config;
use crate::executor::runner::{
    Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs, NodeRunner, UnimplementedRunner,
};
use crate::storage::single_output_path;
use crate::types::{DataType, NodeKind, ScalarType};

/// Dispatches each node kind to its implementation. Kinds without one yet fail with the
/// standard "not implemented" message (see [`UnimplementedRunner`]).
pub struct EngineRunner {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    pub extractor: ExtractorClient,
    pub llm: LlmClient,
    pub embedder: EmbeddingClient,
}

impl EngineRunner {
    pub fn new(db: PgPool, config: Arc<Config>, http: reqwest::Client) -> Self {
        let extractor = ExtractorClient::new(
            http.clone(),
            config.extractor_url.clone(),
            config.extractor_timeout,
        );
        let llm = LlmClient::new(
            http.clone(),
            config.llm_base_url.clone(),
            config.llm_api_key.clone(),
            config.llm_model.clone(),
            config.llm_effort.clone(),
        );
        let embedder = EmbeddingClient::new(
            http.clone(),
            config.embedding_base_url.clone(),
            config.embedding_api_key.clone(),
            config.embedding_model.clone(),
        );
        Self {
            db,
            config,
            http,
            extractor,
            llm,
            embedder,
        }
    }
}

impl NodeRunner for EngineRunner {
    fn run(
        &self,
        ctx: NodeContext,
        inputs: NodeInputs,
    ) -> BoxFuture<'static, Result<NodeOutputs, NodeError>> {
        match ctx.kind {
            NodeKind::Bundle => {
                let db = self.db.clone();
                async move { bundle::run(&db, &ctx).await }.boxed()
            }
            NodeKind::PickPdf | NodeKind::PickJpg => {
                async move { pick::run(&ctx, &inputs) }.boxed()
            }
            NodeKind::Extract => {
                let client = self.extractor.clone();
                let data_dir = self.config.data_dir.clone();
                async move { extract::run(&client, &data_dir, &ctx, &inputs).await }.boxed()
            }
            NodeKind::Translate => {
                let llm = self.llm.clone();
                async move { translate::run(&llm, &ctx, &inputs).await }.boxed()
            }
            NodeKind::Analyze => {
                let llm = self.llm.clone();
                async move { analyze::run(&llm, &ctx, &inputs).await }.boxed()
            }
            NodeKind::Summary => {
                let llm = self.llm.clone();
                async move { summary::run(&llm, &ctx, &inputs).await }.boxed()
            }
            NodeKind::Rag => {
                let db = self.db.clone();
                let embedder = self.embedder.clone();
                async move { rag::run(&db, &embedder, &ctx, &inputs).await }.boxed()
            }
            NodeKind::JsonInput => async move { json_input::run(&ctx).await }.boxed(),
            _ => UnimplementedRunner.run(ctx, inputs),
        }
    }
}

/// Largest image sent to the LLM (5 MiB).
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Reads a UTF-8 text input; `what` names the socket for error messages.
pub(crate) async fn read_text(path: &Path, what: &str) -> Result<String, NodeError> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| NodeError::Failed(format!("입력 {what} 을(를) 읽을 수 없습니다: {e}")))
}

/// Reads an image input, enforcing [`MAX_IMAGE_BYTES`] before loading it.
pub(crate) async fn read_image(path: &Path) -> Result<Vec<u8>, NodeError> {
    let unreadable = |e: std::io::Error| {
        NodeError::Failed(format!(
            "이미지를 읽을 수 없습니다 ({}): {e}",
            path.display()
        ))
    };
    let size = tokio::fs::metadata(path).await.map_err(unreadable)?.len();
    if size > MAX_IMAGE_BYTES {
        return Err(NodeError::Failed(
            "이미지가 너무 큽니다 (최대 5MB)".to_owned(),
        ));
    }
    tokio::fs::read(path).await.map_err(unreadable)
}

/// Writes the node's single `md` output.
pub(crate) async fn write_md(ctx: &NodeContext, text: &str) -> Result<NodeOutputs, NodeError> {
    let path = single_output_path(&ctx.out_dir, "md", ScalarType::Md);
    tokio::fs::write(&path, text).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write md output"))
    })?;
    Ok(NodeOutputs::from([(
        "md".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Md),
            paths: vec![path],
        },
    )]))
}
