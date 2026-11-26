CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE workflows (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    graph       JSONB NOT NULL DEFAULT '{"nodes":[],"edges":[]}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE bundles (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE bundle_files (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id     UUID NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    original_name TEXT NOT NULL,
    stored_path   TEXT NOT NULL,
    size_bytes    BIGINT NOT NULL,
    position      INTEGER NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (bundle_id, position)
);

CREATE TABLE runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    status          TEXT NOT NULL CHECK (status IN ('running','succeeded','failed')),
    graph_snapshot  JSONB NOT NULL,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ
);
CREATE INDEX runs_workflow_started_idx ON runs (workflow_id, started_at DESC);

CREATE TABLE node_runs (
    run_id       UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id      TEXT NOT NULL,
    kind         TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed','skipped')),
    outputs      JSONB,
    error        TEXT,
    started_at   TIMESTAMPTZ,
    finished_at  TIMESTAMPTZ,
    PRIMARY KEY (run_id, node_id)
);

CREATE TABLE rag_indexes (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id           UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id          TEXT NOT NULL,
    embedding_model  TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE rag_chunks (
    id           BIGSERIAL PRIMARY KEY,
    index_id     UUID NOT NULL REFERENCES rag_indexes(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    content      TEXT NOT NULL,
    embedding    vector(1536) NOT NULL
);
CREATE INDEX rag_chunks_index_idx ON rag_chunks (index_id, chunk_index);
CREATE INDEX rag_chunks_embedding_idx ON rag_chunks USING hnsw (embedding vector_cosine_ops);
