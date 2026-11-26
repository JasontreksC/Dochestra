-- T-309: embeddings now come from OpenRouter (baai/bge-m3), which produces 1024 dimensions.
--
-- A pgvector column cannot be converted between dimensions, and vectors from different models are
-- not comparable anyway, so every existing RAG index is discarded here (indexes are rebuilt by
-- running the `rag` node again). rag_chunks rows also go away with their index (ON DELETE CASCADE).
DROP INDEX IF EXISTS rag_chunks_embedding_idx;
DELETE FROM rag_chunks;
DELETE FROM rag_indexes;
ALTER TABLE rag_chunks ALTER COLUMN embedding TYPE vector(1024);
CREATE INDEX rag_chunks_embedding_idx ON rag_chunks USING hnsw (embedding vector_cosine_ops);
