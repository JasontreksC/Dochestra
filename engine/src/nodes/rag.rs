//! `rag` node: chunks Markdown, embeds the chunks, stores them in pgvector and outputs the
//! JSON description of the query API (CONTEXT §4.3).

use serde_json::json;
use sqlx::PgPool;

use super::{read_text, translate::split_chunks};
use crate::clients::embedding::EmbeddingClient;
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::single_output_path;
use crate::types::{DataType, ScalarType};

/// Maximum characters per stored chunk.
pub const MAX_CHUNK_CHARS: usize = 1500;

/// Splits Markdown into retrieval chunks of at most [`MAX_CHUNK_CHARS`] characters.
///
/// Blocks (separated by blank lines; fenced code and tables count as one block) are packed
/// greedily. A single block longer than the limit is hard-split every 1500 characters.
pub fn chunk_markdown(md: &str) -> Vec<String> {
    split_chunks(md, MAX_CHUNK_CHARS)
        .into_iter()
        .flat_map(|chunk| hard_split(&chunk, MAX_CHUNK_CHARS))
        .collect()
}

fn hard_split(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(max_chars)
        .map(|piece| piece.iter().collect())
        .collect()
}

pub async fn run(
    db: &PgPool,
    embedder: &EmbeddingClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = inputs
        .get("md")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 md 이(가) 없습니다".to_owned()))?;
    let markdown = read_text(md_path, "md").await?;
    if markdown.trim().is_empty() {
        return Err(NodeError::Failed("문서 내용이 비어 있습니다".to_owned()));
    }

    let chunks = chunk_markdown(&markdown);
    let embeddings = embedder
        .embed(&chunks)
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;

    let index_id = crate::db::rag::create_index_with_chunks(
        db,
        ctx.run_id,
        &ctx.node_id,
        embedder.model(),
        &chunks,
        &embeddings,
    )
    .await
    .map_err(anyhow::Error::from)?;

    let description = json!({
        "type": "rag_api",
        "indexId": index_id,
        "method": "POST",
        "endpoint": format!("/api/rag/{index_id}/query"),
        "request": {
            "question": "string",
            "topK": "number (선택, 기본 5, 1~20)",
        },
        "chunkCount": chunks.len(),
        "embeddingModel": embedder.model(),
    });
    let bytes = serde_json::to_vec_pretty(&description).map_err(anyhow::Error::from)?;
    let path = single_output_path(&ctx.out_dir, "api", ScalarType::Json);
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write api output"))
    })?;

    Ok(NodeOutputs::from([(
        "api".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Json),
            paths: vec![path],
        },
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_documents_are_a_single_chunk() {
        assert_eq!(
            chunk_markdown("# T\n\nhello\n\nworld"),
            ["# T\n\nhello\n\nworld"]
        );
        assert!(chunk_markdown("").is_empty());
    }

    #[test]
    fn blocks_are_packed_up_to_1500_characters() {
        let a = "a".repeat(1000);
        let b = "b".repeat(1000);
        let small = "c".repeat(400);
        // 1000 + 2 + 400 = 1402 fits; the second 1000-char block starts a new chunk.
        let md = format!("{a}\n\n{small}\n\n{b}");
        let chunks = chunk_markdown(&md);
        assert_eq!(chunks, [format!("{a}\n\n{small}"), b]);
        assert!(chunks.iter().all(|c| c.chars().count() <= MAX_CHUNK_CHARS));
    }

    #[test]
    fn oversized_blocks_are_hard_split_into_1500_character_pieces() {
        let big = "x".repeat(3200);
        let md = format!("intro\n\n{big}\n\noutro");
        let chunks = chunk_markdown(&md);
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0], "intro");
        assert_eq!(chunks[1].chars().count(), 1500);
        assert_eq!(chunks[2].chars().count(), 1500);
        assert_eq!(chunks[3].chars().count(), 200);
        assert_eq!(chunks[4], "outro");
        assert_eq!(chunks[1..4].concat(), big);
    }

    #[test]
    fn code_blocks_and_tables_are_not_split_when_they_fit() {
        let code = format!("```\n{}\n\n{}\n```", "l1".repeat(300), "l2".repeat(300));
        let table = "| a | b |\n|---|---|\n| 1 | 2 |";
        let md = format!("{}\n\n{code}\n\n{table}", "p".repeat(900));
        let chunks = chunk_markdown(&md);
        // The code block (1210 chars) cannot share a chunk with the 900-char paragraph.
        assert!(chunks.contains(&format!("{code}\n\n{table}")), "{chunks:?}");
    }

    #[test]
    fn oversized_code_blocks_are_hard_split() {
        let code = format!("```\n{}\n\n{}\n```", "a".repeat(1000), "b".repeat(1000));
        let chunks = chunk_markdown(&code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.concat(), code);
    }

    #[test]
    fn counts_characters_not_bytes() {
        let korean = "가".repeat(1500);
        let chunks = chunk_markdown(&korean);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], korean);
        let longer = "가".repeat(1501);
        let chunks = chunk_markdown(&longer);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1], "가");
    }
}
