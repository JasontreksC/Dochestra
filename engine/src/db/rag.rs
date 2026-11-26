//! `rag_indexes` / `rag_chunks` queries (pgvector).

use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

/// A stored RAG index (one per `rag` node execution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RagIndex {
    pub id: Uuid,
    pub run_id: Uuid,
    pub node_id: String,
    pub embedding_model: String,
}

/// One retrieved chunk; `score` is cosine similarity (`1 - cosine distance`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub chunk_index: i32,
    pub content: String,
    pub score: f64,
}

/// Creates an index with all its chunks in one transaction. `chunks[i]` is stored with
/// `chunk_index = i` and `embeddings[i]`.
pub async fn create_index_with_chunks(
    db: &PgPool,
    run_id: Uuid,
    node_id: &str,
    embedding_model: &str,
    chunks: &[String],
    embeddings: &[Vec<f32>],
) -> Result<Uuid, sqlx::Error> {
    debug_assert_eq!(chunks.len(), embeddings.len());
    let mut tx = db.begin().await?;
    let index_id = sqlx::query_scalar!(
        "INSERT INTO rag_indexes (run_id, node_id, embedding_model) VALUES ($1, $2, $3) RETURNING id",
        run_id,
        node_id,
        embedding_model
    )
    .fetch_one(&mut *tx)
    .await?;

    for (i, (content, embedding)) in chunks.iter().zip(embeddings).enumerate() {
        let chunk_index = i32::try_from(i).unwrap_or(i32::MAX);
        sqlx::query!(
            "INSERT INTO rag_chunks (index_id, chunk_index, content, embedding) VALUES ($1, $2, $3, $4)",
            index_id,
            chunk_index,
            content,
            Vector::from(embedding.clone()) as Vector
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(index_id)
}

pub async fn get_index(db: &PgPool, id: Uuid) -> Result<Option<RagIndex>, sqlx::Error> {
    sqlx::query_as!(
        RagIndex,
        "SELECT id, run_id, node_id, embedding_model FROM rag_indexes WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// The `top_k` chunks of an index closest to `query` by cosine distance, best first (ties by
/// chunk index).
pub async fn search(
    db: &PgPool,
    index_id: Uuid,
    query: &[f32],
    top_k: i64,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    let mut tx = db.begin().await?;
    // The HNSW index is shared by all rag indexes; without iterative scans the `index_id`
    // filter could be applied after a fixed-size candidate list and return too few rows.
    sqlx::query("SET LOCAL hnsw.iterative_scan = strict_order")
        .execute(&mut *tx)
        .await?;
    let hits = sqlx::query_as!(
        SearchHit,
        r#"SELECT chunk_index, content, (1 - (embedding <=> $2))::float8 AS "score!"
           FROM rag_chunks
           WHERE index_id = $1
           ORDER BY embedding <=> $2, chunk_index
           LIMIT $3"#,
        index_id,
        Vector::from(query.to_vec()) as Vector,
        top_k
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(hits)
}
